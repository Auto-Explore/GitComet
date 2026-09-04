use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::{
    bytes_to_text_preserving_utf8, git_workdir_cmd_for, path_buf_from_git_bytes,
    run_git_raw_output, run_git_simple, run_git_simple_with_paths, validate_hex_commit_id,
    validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, FileStatusKind, StashEntry};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CheckoutRemoteBranchMode, CommitOperationOutcome, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn stash_spec(index: usize) -> String {
    format!("stash@{{{index}}}")
}

fn local_branch_ref_name(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

fn remote_tracking_ref_name(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

fn head_targets_branch(repo: &gix::Repository, branch_ref_name: &str) -> Result<bool> {
    let head_name = repo
        .head_name()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head_name: {e}"))))?;
    Ok(head_name.is_some_and(|name| name.as_bstr() == branch_ref_name))
}

/// The worktree whose HEAD is a local branch, straight from worktree metadata.
enum WorktreeHoldingBranch {
    /// Working directory; validate with `validated_worktree_workdir` before use.
    Workdir(PathBuf),
    /// Private git dir of a linked worktree whose workdir cannot be resolved.
    Unresolved(PathBuf),
}

impl WorktreeHoldingBranch {
    fn display_path(&self) -> &Path {
        match self {
            Self::Workdir(path) | Self::Unresolved(path) => path,
        }
    }
}

fn branch_in_use_worktree_path(
    repo: &gix::Repository,
    branch: &str,
) -> Result<Option<WorktreeHoldingBranch>> {
    // `worktrees()` only lists linked worktrees, so start from the owner repo to
    // include the main worktree even when GitComet is opened from a linked one.
    let owner_repo = repo
        .main_repo()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix main_repo: {e}"))))?;
    let branch_ref_name = local_branch_ref_name(branch);
    if let Some(workdir) = owner_repo.workdir()
        && head_targets_branch(&owner_repo, branch_ref_name.as_str())?
    {
        return Ok(Some(WorktreeHoldingBranch::Workdir(workdir.to_path_buf())));
    }

    let worktrees = owner_repo
        .worktrees()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix worktrees: {e}"))))?;
    for proxy in worktrees {
        let worktree = match proxy.base() {
            Ok(workdir) => WorktreeHoldingBranch::Workdir(workdir),
            Err(_) => WorktreeHoldingBranch::Unresolved(proxy.git_dir().to_path_buf()),
        };
        let linked_repo = proxy
            .into_repo_with_possibly_inaccessible_worktree()
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!("gix open linked worktree: {e}")))
            })?;
        if head_targets_branch(&linked_repo, branch_ref_name.as_str())? {
            return Ok(Some(worktree));
        }
    }

    Ok(None)
}

fn cannot_force_update_branch_error(command: &str, branch: &str, worktree: &Path) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        command,
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        Vec::new(),
        Some(format!(
            "fatal: cannot force update the branch '{branch}' used by worktree at '{}'",
            worktree.display()
        )),
    )))
}

/// Delete `refs/heads/<name>` and its config; the caller checked no worktree holds it.
fn delete_local_branch_ref_and_config(repo: &gix::Repository, name: &str) -> Result<()> {
    let ref_name = local_branch_ref_name(name);
    let Some(reference) = repo
        .try_find_reference(ref_name.as_str())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
    else {
        return Err(delete_branch_force_error(format!(
            "error: branch '{name}' not found"
        )));
    };

    reference
        .delete()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix delete branch {name}: {e}"))))?;
    delete_local_branch_config_section(repo, name)
}

fn delete_local_branch_config_section(repo: &gix::Repository, branch: &str) -> Result<()> {
    edit_local_config(repo, |config| {
        let mut removed = false;
        while config
            .remove_section("branch", Some(branch.into()))
            .is_some()
        {
            removed = true;
        }
        removed
    })
}

/// Drop `branch.<name>.remote` and `.merge` so the branch tracks nothing.
fn clear_local_branch_upstream(repo: &gix::Repository, branch: &str) -> Result<()> {
    edit_local_config(repo, |config| {
        let Ok(mut section) = config.section_mut("branch", Some(gix::bstr::BStr::new(branch)))
        else {
            return false;
        };
        let mut removed = false;
        for key in ["remote", "merge"] {
            while section.remove(key).is_some() {
                removed = true;
            }
        }
        removed
    })
}

/// Apply `edit` to the local config and write it back when it reports a change.
fn edit_local_config(
    repo: &gix::Repository,
    edit: impl FnOnce(&mut gix::config::File) -> bool,
) -> Result<()> {
    let config_path = repo.common_dir().join("config");
    let mut config = match gix::config::File::from_path_no_includes(
        config_path.clone(),
        gix::config::Source::Local,
    ) {
        Ok(config) => config,
        Err(gix::config::file::init::from_paths::Error::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(());
        }
        Err(e) => {
            return Err(Error::new(ErrorKind::Backend(format!(
                "gix read local config {}: {e}",
                config_path.display()
            ))));
        }
    };

    if edit(&mut config) {
        let serialized = config.to_bstring();
        let mut lock = match gix::lock::File::acquire_to_update_resource(
            &config_path,
            gix::lock::acquire::Fail::Immediately,
            None,
        ) {
            Ok(lock) => lock,
            // Git still deletes the branch ref when branch-config cleanup can't
            // take the config lock; it only warns and leaves the config in place.
            Err(gix::lock::acquire::Error::PermanentlyLocked { .. }) => return Ok(()),
            Err(e) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "lock local config {}: {e}",
                    config_path.display()
                ))));
            }
        };
        lock.write_all(serialized.as_ref() as &[u8]).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "write local config {}: {e}",
                config_path.display()
            )))
        })?;
        lock.commit().map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "commit local config {}: {}",
                config_path.display(),
                e.error
            )))
        })?;
    }

    Ok(())
}

fn delete_branch_force_error(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        "git branch -D",
        GitFailureId::CommandFailed,
        Some(1),
        Vec::new(),
        Vec::new(),
        Some(detail.into()),
    )))
}

fn create_branch_error(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        "git branch",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        Vec::new(),
        Some(detail.into()),
    )))
}

fn branch_already_exists_error(command: &str, branch: &str) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        command,
        GitFailureId::BranchAlreadyExists,
        Some(128),
        Vec::new(),
        Vec::new(),
        Some(format!("fatal: a branch named '{branch}' already exists")),
    )))
}

fn checkout_remote_branch_missing_error(remote: &str, branch: &str) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        "git checkout --track",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        Vec::new(),
        Some(format!(
            "Remote branch {remote}/{branch} no longer exists. Refresh remote branches and try again."
        )),
    )))
}

/// `target` is an arbitrary rev-parse expression - a commit id, a tag, a local
/// branch, or a remote-tracking branch the sidebar may have listed before it
/// was pruned. Only the last is worth a refresh hint.
fn branch_target_missing_error(repo: &gix::Repository, target: &str) -> Error {
    let names_remote_branch = repo.remote_names().iter().any(|remote| {
        target
            .strip_prefix(remote.to_str_lossy().as_ref())
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|branch| !branch.is_empty())
    });
    if names_remote_branch {
        create_branch_error(format!(
            "Branch source '{target}' no longer exists. Refresh remote branches and try again."
        ))
    } else {
        create_branch_error(format!("fatal: not a valid object name: '{target}'"))
    }
}

fn resolve_branch_target_commit_id(repo: &gix::Repository, target: &str) -> Result<gix::ObjectId> {
    let object = repo
        .rev_parse_single(target)
        .map_err(|_| branch_target_missing_error(repo, target))?
        .object()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix rev-parse {target} object: {e}"
            )))
        })?;
    let commit = object.peel_to_commit().map_err(|e| match e {
        gix::object::peel::to_kind::Error::NotFound { oid, actual, .. } => create_branch_error(
            format!(
                "error: object {oid} is a {actual}, not a commit\nfatal: not a valid branch point: '{target}'"
            ),
        ),
        other => Error::new(ErrorKind::Backend(format!(
            "gix branch target {target} to commit: {other}"
        ))),
    })?;
    Ok(commit.id)
}

fn resolve_stash_commit(repo: &gix::Repository, index: usize) -> Result<gix::Commit<'_>> {
    let stash_spec = stash_spec(index);
    repo.rev_parse_single(stash_spec.as_str())
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix rev-parse {stash_spec}: {e}"
            )))
        })?
        .object()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash object {stash_spec}: {e}"
            )))
        })?
        .try_into_commit()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash commit {stash_spec}: {e}"
            )))
        })
}

fn stash_untracked_parent_id(stash_commit: &gix::Commit<'_>) -> Option<gix::ObjectId> {
    stash_commit.parent_ids().nth(2).map(|id| id.detach())
}

fn stash_untracked_tree<'repo>(
    repo: &'repo gix::Repository,
    index: usize,
    untracked_parent_id: Option<gix::ObjectId>,
) -> Result<Option<gix::Tree<'repo>>> {
    let stash_spec = stash_spec(index);
    let untracked_parent_id = if let Some(id) = untracked_parent_id {
        id
    } else {
        match stash_untracked_parent_id(&resolve_stash_commit(repo, index)?) {
            Some(id) => id,
            None => return Ok(None),
        }
    };

    repo.find_commit(untracked_parent_id)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash untracked parent {stash_spec}: {e}"
            )))
        })?
        .tree()
        .map(Some)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash untracked tree {stash_spec}: {e}"
            )))
        })
}

fn path_blocks_untracked_restore(workdir: &Path, path: &Path) -> bool {
    let mut candidate = workdir.to_path_buf();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if components.peek().is_none() || !metadata.is_dir() {
                    return true;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return false,
        }
    }
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TreeEntryFingerprint {
    mode: gix::index::entry::Mode,
    id: gix::ObjectId,
}

#[derive(Default)]
struct StashApplyPreflight {
    would_overwrite_worktree_changes: bool,
    untracked_parent_id: Option<gix::ObjectId>,
}

#[derive(Default)]
struct StashUntrackedRestoreConflicts {
    paths: Vec<PathBuf>,
    stashed_contents: FxHashMap<PathBuf, Vec<u8>>,
    untracked_parent_id: Option<gix::ObjectId>,
}

impl StashUntrackedRestoreConflicts {
    fn from_output(stdout: &str, stderr: &str, untracked_parent_id: Option<gix::ObjectId>) -> Self {
        Self {
            paths: untracked_restore_conflict_paths_from_output(stdout, stderr),
            stashed_contents: FxHashMap::default(),
            untracked_parent_id,
        }
    }

    fn has_conflicts(&self) -> bool {
        !self.paths.is_empty()
    }
}

fn paths_may_conflict_on_restore(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn tree_entry_fingerprints(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    context: &str,
) -> Result<FxHashMap<PathBuf, TreeEntryFingerprint>> {
    let index = repo
        .index_from_tree(&tree.id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{context}: {e}"))))?;
    let path_backing = index.path_backing();
    let mut entries = FxHashMap::default();
    for entry in index.entries() {
        let path = path_buf_from_git_bytes(entry.path_in(path_backing).as_ref(), context)?;
        entries.insert(
            path,
            TreeEntryFingerprint {
                mode: entry.mode,
                id: entry.id,
            },
        );
    }
    Ok(entries)
}

fn stash_tracked_change_paths(
    repo: &gix::Repository,
    stash_commit: &gix::Commit<'_>,
    index: usize,
) -> Result<FxHashSet<PathBuf>> {
    let stash_spec = stash_spec(index);

    let Some(base_parent_id) = stash_commit.parent_ids().next().map(|id| id.detach()) else {
        return Ok(FxHashSet::default());
    };

    let base_tree = repo
        .find_commit(base_parent_id)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash base parent {stash_spec}: {e}"
            )))
        })?
        .tree()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash base tree {stash_spec}: {e}"
            )))
        })?;
    let stash_tree = stash_commit.tree().map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix stash tracked tree {stash_spec}: {e}"
        )))
    })?;

    let mut base_entries = tree_entry_fingerprints(repo, &base_tree, "gix stash base tree index")?;
    let mut changed_paths = FxHashSet::default();
    for (path, stash_entry) in
        tree_entry_fingerprints(repo, &stash_tree, "gix stash tracked tree index")?
    {
        match base_entries.remove(&path) {
            Some(base_entry) if base_entry == stash_entry => {}
            _ => {
                changed_paths.insert(path);
            }
        }
    }
    changed_paths.extend(base_entries.into_keys());
    Ok(changed_paths)
}

impl GixRepo {
    fn head_commit_id_for_outcome(&self) -> Result<Option<CommitId>> {
        let repo = self.reopen_repo()?;
        gix_head_id_or_none(&repo).map(|id| id.map(|id| CommitId(id.to_string().into())))
    }

    fn ref_exists_in_repo(repo: &gix::Repository, ref_name: &str) -> Result<bool> {
        Ok(repo
            .try_find_reference(ref_name)
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix try_find_reference {ref_name}: {e}"
                )))
            })?
            .is_some())
    }

    fn local_branch_exists_in_repo(repo: &gix::Repository, branch: &str) -> Result<bool> {
        let ref_name = local_branch_ref_name(branch);
        Self::ref_exists_in_repo(repo, ref_name.as_str())
    }

    fn local_branch_exists(&self, branch: &str) -> Result<bool> {
        // Branch creation/checkouts can happen while the backend stays open or
        // while another Git process races with us, so always read ref existence
        // from a fresh repo snapshot before deciding how to proceed.
        let repo = self.reopen_repo()?;
        Self::local_branch_exists_in_repo(&repo, branch)
    }

    fn remote_tracking_branch_exists_in_repo(
        repo: &gix::Repository,
        remote: &str,
        branch: &str,
    ) -> Result<bool> {
        let ref_name = remote_tracking_ref_name(remote, branch);
        Self::ref_exists_in_repo(repo, ref_name.as_str())
    }

    fn create_local_branch_reference(&self, branch: &str, target: &str) -> Result<()> {
        let mut repo = self.reopen_repo()?;
        if Self::local_branch_exists_in_repo(&repo, branch)? {
            return Err(branch_already_exists_error("git branch", branch));
        }

        let target_id = resolve_branch_target_commit_id(&repo, target)?;
        repo.committer_or_set_generic_fallback()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix committer fallback: {e}"))))?;

        if let Err(e) = repo.reference(
            local_branch_ref_name(branch),
            target_id,
            gix::refs::transaction::PreviousValue::MustNotExist,
            format!("branch: Created from {target}"),
        ) {
            if self.local_branch_exists(branch)? {
                return Err(branch_already_exists_error("git branch", branch));
            }
            return Err(Error::new(ErrorKind::Backend(format!(
                "gix create branch {branch}: {e}"
            ))));
        }
        Ok(())
    }

    pub(super) fn create_branch_impl(&self, name: &str, target: &CommitId) -> Result<()> {
        validate_ref_like_arg(name, "branch name")?;
        validate_ref_like_arg(target.as_ref(), "branch target")?;
        self.create_local_branch_reference(name, target.as_ref())
    }

    pub(super) fn rename_branch_impl(&self, old_name: &str, new_name: &str) -> Result<()> {
        validate_ref_like_arg(old_name, "branch name")?;
        validate_ref_like_arg(new_name, "branch name")?;
        if self.local_branch_exists(new_name)? {
            return Err(branch_already_exists_error("git branch -m", new_name));
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("branch")
            .arg("-m")
            .arg("--")
            .arg(old_name)
            .arg(new_name);
        match run_git_simple(cmd, "git branch -m") {
            Ok(()) => Ok(()),
            // Another Git process can create the branch after the preflight.
            Err(_) if self.local_branch_exists(new_name)? => {
                Err(branch_already_exists_error("git branch -m", new_name))
            }
            Err(err) => Err(err),
        }
    }

    fn is_own_workdir(&self, path: &Path) -> bool {
        path == self.spec.workdir
            || canonicalize_or_original(path.to_path_buf())
                == canonicalize_or_original(self.spec.workdir.clone())
    }

    /// Canonical workdir of a worktree holding `branch`, proven to belong to
    /// this repository. Worktree metadata is untrusted input: git runs in this
    /// path and the app opens it as a tab.
    fn validated_worktree_workdir(
        &self,
        repo: &gix::Repository,
        worktree: WorktreeHoldingBranch,
        branch: &str,
    ) -> Result<PathBuf> {
        let WorktreeHoldingBranch::Workdir(path) = worktree else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "the worktree holding branch '{branch}' at '{}' is inaccessible",
                worktree.display_path().display()
            ))));
        };
        let path = canonicalize_or_original(path);
        if !path.is_dir() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "the worktree holding branch '{branch}' at '{}' does not exist",
                path.display()
            ))));
        }
        let holder = crate::open::open_worktree_repo(&path)
            .map_err(|e| crate::open::map_open_error(e, "gix open worktree holding branch"))?;
        let same_repo = canonicalize_or_original(holder.common_dir().to_path_buf())
            == canonicalize_or_original(repo.common_dir().to_path_buf());
        if !same_repo {
            return Err(Error::new(ErrorKind::Backend(format!(
                "the worktree at '{}' does not belong to this repository",
                path.display()
            ))));
        }
        Ok(path)
    }

    pub(super) fn branch_checked_out_in_other_worktree_impl(
        &self,
        name: &str,
    ) -> Result<Option<PathBuf>> {
        validate_ref_like_arg(name, "branch name")?;
        let repo = self.reopen_repo()?;
        let Some(worktree) = branch_in_use_worktree_path(&repo, name)? else {
            return Ok(None);
        };
        if matches!(&worktree, WorktreeHoldingBranch::Workdir(path) if self.is_own_workdir(path)) {
            return Ok(None);
        }
        let path = self.validated_worktree_workdir(&repo, worktree, name)?;
        Ok((!self.is_own_workdir(&path)).then_some(path))
    }

    pub(super) fn rename_branch_force_impl(&self, old_name: &str, new_name: &str) -> Result<()> {
        validate_ref_like_arg(old_name, "branch name")?;
        validate_ref_like_arg(new_name, "branch name")?;
        if old_name == new_name {
            return Err(Error::new(ErrorKind::Backend(
                "invalid branch rename: old and new names are the same".to_string(),
            )));
        }

        let repo = self.reopen_repo()?;
        let Some(worktree) = branch_in_use_worktree_path(&repo, new_name)? else {
            let mut cmd = self.git_workdir_cmd();
            cmd.arg("branch")
                .arg("-M")
                .arg("--")
                .arg(old_name)
                .arg(new_name);
            return run_git_simple(cmd, "git branch -M");
        };
        if !matches!(&worktree, WorktreeHoldingBranch::Workdir(path) if self.is_own_workdir(path)) {
            return Err(cannot_force_update_branch_error(
                "git branch -M",
                new_name,
                worktree.display_path(),
            ));
        }

        // `new_name` is HEAD here: git refuses `-M`, so reset it in place instead.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("checkout")
            .arg("--no-track")
            .arg("-B")
            .arg(new_name)
            .arg(old_name);
        run_git_simple(cmd, "git checkout -B")?;

        let repo = self.reopen_repo()?;
        if let Some(worktree) = branch_in_use_worktree_path(&repo, old_name)? {
            let path = self.validated_worktree_workdir(&repo, worktree, old_name)?;
            if self.is_own_workdir(&path) {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "branch '{old_name}' is unexpectedly still checked out here"
                ))));
            }
            let mut cmd = git_workdir_cmd_for(&path);
            cmd.arg("checkout").arg("--detach");
            run_git_simple(cmd, "git checkout --detach")?;
        }
        delete_local_branch_ref_and_config(&self.reopen_repo()?, old_name)
    }

    pub(super) fn delete_branch_impl(&self, name: &str) -> Result<()> {
        validate_ref_like_arg(name, "branch name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("branch").arg("-d").arg("--").arg(name);
        run_git_simple(cmd, "git branch -d")
    }

    pub(super) fn delete_branch_force_impl(&self, name: &str) -> Result<()> {
        validate_ref_like_arg(name, "branch name")?;

        let repo = self.reopen_repo()?;
        if let Some(worktree) = branch_in_use_worktree_path(&repo, name)? {
            return Err(delete_branch_force_error(format!(
                "error: cannot delete branch '{name}' used by worktree at '{}'",
                worktree.display_path().display()
            )));
        }
        delete_local_branch_ref_and_config(&repo, name)
    }

    pub(super) fn checkout_branch_impl(&self, name: &str) -> Result<()> {
        validate_ref_like_arg(name, "branch name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("checkout").arg(name);
        run_git_simple(cmd, "git checkout")
    }

    /// `git checkout --no-track -B <name> <target>`: create the branch at
    /// `target`, or reset an existing branch of that name to `target`, then
    /// check it out. Also handles the branch checked out in this worktree,
    /// which a `git branch -f` + `git checkout` pair cannot. Like a fresh
    /// branch, the result tracks nothing: any upstream the existing branch had
    /// is cleared.
    pub(super) fn create_branch_force_and_checkout_impl(
        &self,
        name: &str,
        target: &CommitId,
    ) -> Result<()> {
        validate_ref_like_arg(name, "branch name")?;
        validate_ref_like_arg(target.as_ref(), "branch target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("checkout")
            .arg("--no-track")
            .arg("-B")
            .arg(name)
            .arg(target.as_ref());
        run_git_simple(cmd, "git checkout -B")?;
        clear_local_branch_upstream(&self.reopen_repo()?, name)
    }

    pub(super) fn checkout_remote_branch_impl(
        &self,
        remote: &str,
        branch: &str,
        local_branch: &str,
        mode: CheckoutRemoteBranchMode,
    ) -> Result<()> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;
        validate_ref_like_arg(local_branch, "branch name")?;

        let repo = self.reopen_repo()?;
        if !Self::remote_tracking_branch_exists_in_repo(&repo, remote, branch)? {
            return Err(checkout_remote_branch_missing_error(remote, branch));
        }

        let upstream = format!("{remote}/{branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("checkout")
            .arg("--track")
            .arg(match mode {
                CheckoutRemoteBranchMode::Create => "-b",
                CheckoutRemoteBranchMode::Overwrite => "-B",
            })
            .arg(local_branch)
            .arg(&upstream);
        run_git_simple(cmd, "git checkout --track")
    }

    pub(super) fn checkout_commit_impl(&self, id: &CommitId) -> Result<()> {
        validate_hex_commit_id(id)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("checkout").arg(id.as_ref());
        run_git_simple(cmd, "git checkout <commit>")
    }

    pub(super) fn cherry_pick_impl(&self, id: &CommitId) -> Result<()> {
        validate_hex_commit_id(id)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cherry-pick").arg("--").arg(id.as_ref());
        run_git_simple(cmd, "git cherry-pick")
    }

    pub(super) fn revert_impl(&self, id: &CommitId) -> Result<()> {
        validate_hex_commit_id(id)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("revert")
            .arg("--no-edit")
            .arg("--")
            .arg(id.as_ref());
        run_git_simple(cmd, "git revert")
    }

    pub(super) fn stash_create_impl(&self, message: &str, include_untracked: bool) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("stash").arg("push");
        if include_untracked {
            cmd.arg("-u");
        }
        if !message.is_empty() {
            cmd.arg("-m").arg(message);
        }
        run_git_simple(cmd, "git stash push")
    }

    pub(super) fn stash_list_impl(&self) -> Result<Vec<StashEntry>> {
        let repo = self._repo.to_thread_local();
        super::log::stash_reflog_entries(&repo)
    }

    pub(super) fn stash_apply_impl(&self, index: usize) -> Result<()> {
        let preflight = self.stash_apply_preflight(index);
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("core.quotePath=false")
            .arg("stash")
            .arg("apply")
            .arg(stash_spec(index));
        let output = run_git_raw_output(cmd, "git stash apply")?;

        if output.status.success() {
            return Ok(());
        }

        let stdout = bytes_to_text_preserving_utf8(&output.stdout);
        let stderr = bytes_to_text_preserving_utf8(&output.stderr);
        let untracked_restore_conflicts = if preflight.would_overwrite_worktree_changes {
            StashUntrackedRestoreConflicts::default()
        } else {
            self.stash_untracked_restore_conflicts(
                index,
                preflight.untracked_parent_id,
                &stdout,
                &stderr,
            )
        };
        let failure_id = stash_apply_failure_id(
            preflight.would_overwrite_worktree_changes,
            untracked_restore_conflicts.has_conflicts(),
        );

        if failure_id == GitFailureId::UntrackedRestoreConflict {
            // Best-effort merge markers for collided untracked files should never
            // mask the original stash-apply failure we want to surface.
            let _ = self
                .merge_untracked_restore_conflicts_from_stash(index, untracked_restore_conflicts);
        }

        let detail = stash_apply_failure_detail(&stdout, &stderr);
        Err(Error::new(ErrorKind::Git(GitFailure::new(
            "git stash apply",
            failure_id,
            output.status.code(),
            output.stdout,
            output.stderr,
            detail,
        ))))
    }

    pub(super) fn stash_drop_impl(&self, index: usize) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("stash").arg("drop").arg(stash_spec(index));
        run_git_simple(cmd, "git stash drop")
    }

    pub(super) fn stage_impl(&self, paths: &[&Path]) -> Result<()> {
        run_git_simple_with_paths(&self.spec.workdir, "git add", &["add", "-A"], paths)
    }

    fn merge_untracked_restore_conflicts_from_stash(
        &self,
        index: usize,
        conflicts: StashUntrackedRestoreConflicts,
    ) -> Result<()> {
        let mut stashed_contents = conflicts.stashed_contents;
        if conflicts
            .paths
            .iter()
            .any(|path| !stashed_contents.contains_key(path))
        {
            let repo = self.reopen_repo()?;
            let tree = stash_untracked_tree(&repo, index, conflicts.untracked_parent_id)?;
            for path in &conflicts.paths {
                if stashed_contents.contains_key(path) {
                    continue;
                }
                stashed_contents.insert(
                    path.clone(),
                    stash_untracked_blob_bytes(tree.as_ref(), path)?,
                );
            }
        }

        for path in conflicts.paths {
            let ours_path = self.spec.workdir.join(&path);
            if !ours_path.exists() {
                continue;
            }

            let ours_bytes =
                fs::read(&ours_path).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            let theirs_bytes = stashed_contents.remove(&path).ok_or_else(|| {
                stash_apply_error(
                    GitFailureId::UntrackedRestoreConflict,
                    format!("could not read stashed untracked file {}", path.display()),
                )
            })?;
            if ours_bytes == theirs_bytes {
                continue;
            }

            let ours_text = std::str::from_utf8(&ours_bytes).map_err(|_| {
                stash_apply_error(
                    GitFailureId::UntrackedRestoreConflict,
                    format!(
                        "cannot merge binary/local non-utf8 untracked file {}",
                        path.display()
                    ),
                )
            })?;
            let theirs_text = std::str::from_utf8(&theirs_bytes).map_err(|_| {
                stash_apply_error(
                    GitFailureId::UntrackedRestoreConflict,
                    format!(
                        "cannot merge binary/stashed non-utf8 untracked file {}",
                        path.display()
                    ),
                )
            })?;

            let merged_text = build_untracked_conflict_markers(ours_text, theirs_text);
            fs::write(&ours_path, merged_text).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        }

        Ok(())
    }

    fn stash_untracked_restore_conflicts(
        &self,
        index: usize,
        untracked_parent_id: Option<gix::ObjectId>,
        stdout: &str,
        stderr: &str,
    ) -> StashUntrackedRestoreConflicts {
        self.try_stash_untracked_restore_conflicts(index, untracked_parent_id)
            .unwrap_or_else(|_| {
                StashUntrackedRestoreConflicts::from_output(stdout, stderr, untracked_parent_id)
            })
    }

    fn try_stash_untracked_restore_conflicts(
        &self,
        index: usize,
        untracked_parent_id: Option<gix::ObjectId>,
    ) -> Result<StashUntrackedRestoreConflicts> {
        let repo = self.reopen_repo()?;
        let Some(tree) = stash_untracked_tree(&repo, index, untracked_parent_id)? else {
            return Ok(StashUntrackedRestoreConflicts {
                untracked_parent_id,
                ..Default::default()
            });
        };

        let mut conflicts =
            stash_untracked_restore_conflicts_from_tree(&repo, &tree, &self.spec.workdir)?;
        conflicts.untracked_parent_id = untracked_parent_id;
        Ok(conflicts)
    }

    fn worktree_overwrite_blocker_paths(&self) -> Result<FxHashSet<PathBuf>> {
        let status = self.status_impl()?;
        Ok(status
            .unstaged
            .into_iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    FileStatusKind::Modified
                        | FileStatusKind::Added
                        | FileStatusKind::Deleted
                        | FileStatusKind::Renamed
                        // `git stash apply` also refuses to start when tracked
                        // stash payload would overwrite an untracked path.
                        | FileStatusKind::Untracked
                )
            })
            .map(|entry| entry.path)
            .collect())
    }

    fn stash_apply_preflight(&self, index: usize) -> StashApplyPreflight {
        self.try_stash_apply_preflight(index).unwrap_or_default()
    }

    fn try_stash_apply_preflight(&self, index: usize) -> Result<StashApplyPreflight> {
        let worktree_overwrite_blockers = self.worktree_overwrite_blocker_paths()?;
        if worktree_overwrite_blockers.is_empty() {
            return Ok(StashApplyPreflight::default());
        }

        let repo = self.reopen_repo()?;
        let stash_commit = resolve_stash_commit(&repo, index)?;
        let tracked_change_paths = stash_tracked_change_paths(&repo, &stash_commit, index)?;

        let would_overwrite_worktree_changes = tracked_change_paths.iter().any(|restore_path| {
            worktree_overwrite_blockers
                .iter()
                .any(|dirty_path| paths_may_conflict_on_restore(dirty_path, restore_path))
        });

        Ok(StashApplyPreflight {
            would_overwrite_worktree_changes,
            untracked_parent_id: stash_untracked_parent_id(&stash_commit),
        })
    }

    pub(super) fn unstage_impl(&self, paths: &[&Path]) -> Result<()> {
        let repo = self._repo.to_thread_local();
        let has_commits = super::history::gix_head_id_or_none(&repo)?.is_some();

        // Unstaging changes what is staged. It must never touch a merge in
        // progress: resetting an unmerged path collapses its stage 1/2/3 entries
        // to HEAD, so the file stops being conflicted and reappears as an
        // ordinary modification still full of conflict markers — and a bare
        // `git reset` additionally clears MERGE_HEAD, silently aborting the
        // merge. Conflicted paths are therefore left exactly as they are.
        let conflicted: FxHashSet<PathBuf> = super::status::gix_unmerged_conflicts(&repo)?
            .into_iter()
            .map(|(path, _)| path)
            .collect();

        if paths.is_empty() {
            if has_commits {
                if conflicted.is_empty() {
                    let mut cmd = self.git_workdir_cmd();
                    cmd.arg("reset");
                    return run_git_simple(cmd, "git reset");
                }

                // Enumerated from the index rather than from the status list:
                // the latter reports a staged rename as its destination alone,
                // and resetting only that half leaves the source path's staged
                // deletion behind. Conflicted paths are excluded there too, so
                // this is exactly the set "unstage everything" may act on — and
                // a path-limited `git reset` only rewrites those index entries,
                // leaving MERGE_HEAD and every worktree file untouched.
                let staged = self.staged_index_paths_impl()?;
                let staged_paths: Vec<&Path> = staged.iter().map(|path| path.as_path()).collect();
                if staged_paths.is_empty() {
                    return Ok(());
                }
                return run_git_simple_with_paths(
                    &self.spec.workdir,
                    "git reset HEAD",
                    &["reset", "HEAD"],
                    &staged_paths,
                );
            }

            let mut cmd = self.git_workdir_cmd();
            cmd.arg("rm").arg("--cached").arg("-r").arg("--").arg(".");
            return run_git_simple(cmd, "git rm --cached -r");
        }

        let requested: Vec<&Path> = paths
            .iter()
            .copied()
            .filter(|path| !conflicted.contains(*path))
            .collect();
        if requested.is_empty() {
            return Ok(());
        }

        if has_commits {
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git reset HEAD",
                &["reset", "HEAD"],
                &requested,
            )
        } else {
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git rm --cached",
                &["rm", "--cached"],
                &requested,
            )
        }
    }

    pub(super) fn commit_impl(&self, message: &str) -> Result<()> {
        let merge_in_progress = self.merge_in_progress_for_commit()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit");
        if merge_in_progress {
            cmd.arg("--allow-empty");
        }
        cmd.arg("-m").arg(message);
        let label = if merge_in_progress {
            "git commit --allow-empty"
        } else {
            "git commit"
        };
        run_git_simple(cmd, label)
    }

    pub(super) fn commit_with_outcome_impl(&self, message: &str) -> Result<CommitOperationOutcome> {
        let local_branch = self.current_branch_name_for_outcome()?;
        let pre_head = self.head_commit_id_for_outcome()?;
        self.commit_impl(message)?;
        let post_head = self.head_commit_id_for_outcome()?;
        Ok(CommitOperationOutcome {
            local_branch,
            pre_head,
            post_head,
        })
    }

    fn merge_in_progress_for_commit(&self) -> Result<bool> {
        let repo = self._repo.to_thread_local();
        Ok(repo.state() == Some(gix::state::InProgress::Merge))
    }

    pub(super) fn commit_amend_impl(&self, message: &str) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit").arg("--amend").arg("-m").arg(message);
        run_git_simple(cmd, "git commit --amend")
    }

    pub(super) fn commit_amend_with_outcome_impl(
        &self,
        message: &str,
    ) -> Result<CommitOperationOutcome> {
        let local_branch = self.current_branch_name_for_outcome()?;
        let pre_head = self.head_commit_id_for_outcome()?;
        self.commit_amend_impl(message)?;
        let post_head = self.head_commit_id_for_outcome()?;
        Ok(CommitOperationOutcome {
            local_branch,
            pre_head,
            post_head,
        })
    }
}

impl GixRepo {
    fn current_branch_name_for_outcome(&self) -> Result<Option<String>> {
        let head = self.current_branch_impl()?;
        let head = head.trim();
        if head.is_empty() || head == "HEAD" {
            return Ok(None);
        }
        Ok(Some(head.to_string()))
    }
}

fn stash_untracked_restore_conflicts_from_tree(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    workdir: &Path,
) -> Result<StashUntrackedRestoreConflicts> {
    let tree_index = repo.index_from_tree(&tree.id).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix stash untracked index: {e}"
        )))
    })?;
    let path_backing = tree_index.path_backing();

    let mut conflicts = StashUntrackedRestoreConflicts::default();
    for entry in tree_index.entries() {
        let path = path_buf_from_git_bytes(
            entry.path_in(path_backing).as_ref(),
            "gix stash untracked path",
        )?;
        if !path_blocks_untracked_restore(workdir, &path) {
            continue;
        }

        conflicts
            .stashed_contents
            .insert(path.clone(), stash_untracked_blob_bytes(Some(tree), &path)?);
        conflicts.paths.push(path);
    }

    Ok(conflicts)
}

fn stash_untracked_blob_bytes(tree: Option<&gix::Tree<'_>>, path: &Path) -> Result<Vec<u8>> {
    let Some(tree) = tree else {
        return Err(stash_apply_error(
            GitFailureId::UntrackedRestoreConflict,
            format!("could not read stashed untracked file {}", path.display()),
        ));
    };
    let Some(entry) = tree.lookup_entry_by_path(path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix stash untracked lookup {}: {e}",
            path.display()
        )))
    })?
    else {
        return Err(stash_apply_error(
            GitFailureId::UntrackedRestoreConflict,
            format!("could not read stashed untracked file {}", path.display()),
        ));
    };

    let mut blob = entry
        .object()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash untracked object {}: {e}",
                path.display()
            )))
        })?
        .try_into_blob()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix stash untracked blob {}: {e}",
                path.display()
            )))
        })?;
    Ok(blob.take_data())
}

fn stash_apply_failure_id(
    would_overwrite_tracked_changes: bool,
    has_untracked_restore_conflicts: bool,
) -> GitFailureId {
    if would_overwrite_tracked_changes {
        GitFailureId::WorktreeWouldBeOverwritten
    } else if has_untracked_restore_conflicts {
        GitFailureId::UntrackedRestoreConflict
    } else {
        GitFailureId::StashApplyConflict
    }
}

fn stash_apply_failure_detail(stdout: &str, stderr: &str) -> Option<String> {
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    (!detail.is_empty()).then(|| detail.to_string())
}

fn stash_apply_error(id: GitFailureId, detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        "git stash apply",
        id,
        None,
        Vec::new(),
        Vec::new(),
        Some(detail.into()),
    )))
}

fn untracked_restore_conflict_paths_from_output(stdout: &str, stderr: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = FxHashSet::default();
    let suffix = " already exists, no checkout";
    for line in stderr.lines().chain(stdout.lines()) {
        let Some(mut path) = line.trim().strip_suffix(suffix) else {
            continue;
        };
        path = path.trim();
        if let Some(stripped) = path.strip_prefix("error: ") {
            path = stripped.trim();
        } else if let Some(stripped) = path.strip_prefix("fatal: ") {
            path = stripped.trim();
        }
        if let Some(stripped) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
            path = stripped;
        } else if let Some(stripped) = path.strip_prefix('\'').and_then(|p| p.strip_suffix('\'')) {
            path = stripped;
        }
        if path.is_empty() {
            continue;
        }
        if seen.insert(path.to_string()) {
            out.push(PathBuf::from(path));
        }
    }
    out
}

fn build_untracked_conflict_markers(current: &str, stashed: &str) -> String {
    let mut out = String::new();
    out.push_str("<<<<<<< Current file\n");
    out.push_str(current);
    if !current.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("=======\n");
    out.push_str(stashed);
    if !stashed.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(">>>>>>> Stashed file\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{
        build_untracked_conflict_markers, path_blocks_untracked_restore,
        paths_may_conflict_on_restore, stash_apply_failure_id,
        untracked_restore_conflict_paths_from_output,
    };
    use gitcomet_core::error::GitFailureId;
    use std::{fs, path::Path};

    #[test]
    fn parses_untracked_restore_conflict_paths_with_optional_error_prefixes() {
        let stderr = "error: Cargo.toml.orig already exists, no checkout\n";
        let paths = untracked_restore_conflict_paths_from_output("", stderr);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], Path::new("Cargo.toml.orig"));
    }

    #[test]
    fn parses_untracked_restore_conflict_paths_with_quoted_paths() {
        let stderr = "\"docs/a file.txt\" already exists, no checkout\n";
        let paths = untracked_restore_conflict_paths_from_output("", stderr);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], Path::new("docs/a file.txt"));
    }

    #[test]
    fn stash_apply_failure_id_classifies_preflight_cases() {
        assert_eq!(
            stash_apply_failure_id(true, true),
            GitFailureId::WorktreeWouldBeOverwritten
        );
        assert_eq!(
            stash_apply_failure_id(true, false),
            GitFailureId::WorktreeWouldBeOverwritten
        );
        assert_eq!(
            stash_apply_failure_id(false, true),
            GitFailureId::UntrackedRestoreConflict
        );
        assert_eq!(
            stash_apply_failure_id(false, false),
            GitFailureId::StashApplyConflict
        );
    }

    #[test]
    fn untracked_restore_conflict_paths_dedups_and_skips_empty_entries() {
        let stderr = concat!(
            "fatal: 'docs/a file.txt' already exists, no checkout\n",
            "error: 'docs/a file.txt' already exists, no checkout\n",
            "\"\" already exists, no checkout\n",
        );
        let stdout = "docs/b file.txt already exists, no checkout\n";
        let paths = untracked_restore_conflict_paths_from_output(stdout, stderr);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], Path::new("docs/a file.txt"));
        assert_eq!(paths[1], Path::new("docs/b file.txt"));
    }

    #[test]
    fn path_blocks_untracked_restore_detects_existing_path_and_file_ancestor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        fs::write(workdir.join("existing.txt"), "existing").expect("write existing file");
        fs::write(workdir.join("docs"), "file ancestor").expect("write blocking ancestor");

        assert!(path_blocks_untracked_restore(
            workdir,
            Path::new("existing.txt"),
        ));
        assert!(path_blocks_untracked_restore(
            workdir,
            Path::new("docs/a.txt"),
        ));
        assert!(!path_blocks_untracked_restore(
            workdir,
            Path::new("nested/missing.txt"),
        ));
    }

    #[test]
    fn paths_may_conflict_on_restore_matches_equal_or_prefix_related_paths() {
        assert!(paths_may_conflict_on_restore(
            Path::new("a.txt"),
            Path::new("a.txt"),
        ));
        assert!(paths_may_conflict_on_restore(
            Path::new("docs"),
            Path::new("docs/a.txt"),
        ));
        assert!(paths_may_conflict_on_restore(
            Path::new("docs/a.txt"),
            Path::new("docs"),
        ));
        assert!(!paths_may_conflict_on_restore(
            Path::new("docs/a.txt"),
            Path::new("docs/b.txt"),
        ));
    }

    #[test]
    fn build_untracked_conflict_markers_appends_missing_newlines() {
        let merged = build_untracked_conflict_markers("ours", "theirs");
        assert_eq!(
            merged,
            concat!(
                "<<<<<<< Current file\n",
                "ours\n",
                "=======\n",
                "theirs\n",
                ">>>>>>> Stashed file\n"
            )
        );
    }
}
