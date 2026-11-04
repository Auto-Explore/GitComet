//! First-level submodule facts: which worktree a checkout belongs to, and how
//! far each direct submodule's checkout has drifted from what its worktree
//! records.

use super::GixRepo;
use crate::util::{
    git_workdir_cmd_for, path_buf_from_git_bytes, push_literal_paths, run_git_background_output,
    run_git_simple,
};
use gitcomet_core::domain::{
    CommitId, DiscardOutcome, FileStatusKind, Submodule, SubmoduleCheckoutPolicy,
    SubmoduleCheckoutRisk, SubmoduleCheckoutUpdate, SubmoduleDrift, SubmoduleParent,
    SubmodulePointerSnapshot, SubmoduleStatus, short_commit_id,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{
    CancellationToken, CommandOutput, CommitOperationOutcome, Result, SubmoduleTrustDecision,
};
use std::path::{Path, PathBuf};

impl GixRepo {
    pub(super) fn superproject_impl(&self) -> Result<Option<SubmoduleParent>> {
        let workdir = canonicalize_or_original(self.spec.workdir.clone());
        let Some(parent) = superproject_working_tree(&workdir)? else {
            return Ok(None);
        };
        let Ok(path) = workdir.strip_prefix(&parent) else {
            return Ok(None);
        };
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        let path = path.to_path_buf();
        // Only the direct parent is asked about: one more level up is enough
        // to tell a first-level submodule from a deeper one.
        let nested = superproject_working_tree(&parent)?.is_some();
        Ok(Some(SubmoduleParent {
            workdir: parent,
            path,
            nested,
        }))
    }
}

/// One gitlink a discard restores: where its pointer goes and the row that
/// says what is checked out now.
struct GitlinkRestore {
    path: PathBuf,
    staged: bool,
    target: CommitId,
    row: Submodule,
}

impl GixRepo {
    pub(super) fn discard_worktree_changes_with_submodules_impl(
        &self,
        paths: &[&Path],
        policy: SubmoduleCheckoutPolicy,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<DiscardOutcome> {
        if paths.is_empty() {
            return Ok(DiscardOutcome::default());
        }
        if !self.spec.workdir.join(".gitmodules").is_file() {
            self.discard_worktree_changes_impl(paths)?;
            return Ok(DiscardOutcome::default());
        }
        let rows = self.submodule_rows_impl()?;
        if !paths
            .iter()
            .any(|path| rows.iter().any(|row| !row.nested && row.path == *path))
        {
            self.discard_worktree_changes_impl(paths)?;
            return Ok(DiscardOutcome::default());
        }
        let status = self.status_impl()?;
        let repo = self.reopen_repo()?;
        let kind_in = |entries: &[gitcomet_core::domain::FileStatus], path: &Path| {
            entries
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| entry.kind)
        };

        let mut restores = Vec::new();
        let mut other: Vec<&Path> = Vec::new();
        for path in paths.iter().copied() {
            let Some(row) = rows.iter().find(|row| !row.nested && row.path == path) else {
                other.push(path);
                continue;
            };
            let staged = kind_in(&status.staged, path);
            let unstaged = kind_in(&status.unstaged, path);
            // Conflicts keep their refusal, and a gitlink new in the index is
            // removed, exactly as before.
            if row.status == SubmoduleStatus::MergeConflict
                || matches!(
                    staged,
                    Some(FileStatusKind::Added | FileStatusKind::Conflicted)
                )
                || matches!(unstaged, Some(FileStatusKind::Conflicted))
            {
                other.push(path);
                continue;
            }
            if staged.is_none() && unstaged.is_none() {
                continue;
            }
            let target = if staged.is_some() {
                match super::submodules::head_gitlink_commit_id(&repo, path)? {
                    Some(target) => target,
                    None => {
                        other.push(path);
                        continue;
                    }
                }
            } else {
                row.recorded_head.clone()
            };
            restores.push(GitlinkRestore {
                path: path.to_path_buf(),
                staged: staged.is_some(),
                target,
                row: row.clone(),
            });
        }

        let risks = if policy == SubmoduleCheckoutPolicy::KeepCheckout {
            Vec::new()
        } else {
            fill_inner_changes(
                &self.spec.workdir,
                restores.iter_mut().map(|restore| &mut restore.row),
            )?;
            restores
                .iter()
                .filter_map(|restore| self.checkout_risk(&restore.row, &restore.target))
                .collect::<Vec<_>>()
        };
        if policy == SubmoduleCheckoutPolicy::Guarded && !risks.is_empty() {
            return Ok(DiscardOutcome {
                blocked: risks,
                ..DiscardOutcome::default()
            });
        }

        if !other.is_empty() {
            self.discard_worktree_changes_impl(&other)?;
        }
        let mut outcome = DiscardOutcome::default();
        for restore in restores {
            if restore.staged {
                let mut reset = self.git_workdir_cmd();
                reset.arg("reset").arg("-q");
                push_literal_paths(&mut reset, [&restore.path]);
                run_git_simple(reset, "git reset")?;
            }
            if policy == SubmoduleCheckoutPolicy::KeepCheckout {
                continue;
            }
            if restore.row.checked_out_head.is_none() {
                if restore.row.status == SubmoduleStatus::NotInitialized {
                    match self.check_submodule_load_trust_impl(&restore.path, remote_url_policy)? {
                        SubmoduleTrustDecision::Proceed => {
                            self.load_submodule_with_output_impl(
                                &restore.path,
                                &[],
                                remote_url_policy,
                            )?;
                            outcome.updated.push(SubmoduleCheckoutUpdate {
                                path: restore.path,
                                commit: restore.target,
                            });
                        }
                        SubmoduleTrustDecision::Prompt { .. } => {
                            outcome.not_loaded.push(restore.path);
                        }
                    }
                }
                continue;
            }
            let risk = risks.iter().find(|risk| risk.path == restore.path);
            self.check_out_submodule_at(&restore.row, &restore.target, risk, &mut outcome)?;
        }
        Ok(outcome)
    }

    pub(super) fn update_submodule_to_recorded_impl(
        &self,
        path: &Path,
        policy: SubmoduleCheckoutPolicy,
    ) -> Result<DiscardOutcome> {
        let rows = self.submodule_rows_impl()?;
        let Some(mut row) = rows.into_iter().find(|row| !row.nested && row.path == path) else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "'{}' is not a submodule of this repository",
                path.display()
            ))));
        };
        if row.checked_out_head.is_none() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not loaded",
                path.display()
            ))));
        }
        let target = row.recorded_head.clone();
        if policy != SubmoduleCheckoutPolicy::KeepCheckout {
            fill_inner_changes(&self.spec.workdir, [&mut row])?;
        }
        let risk = (policy != SubmoduleCheckoutPolicy::KeepCheckout)
            .then(|| self.checkout_risk(&row, &target))
            .flatten();
        if policy == SubmoduleCheckoutPolicy::Guarded
            && let Some(risk) = risk
        {
            return Ok(DiscardOutcome {
                blocked: vec![risk],
                ..DiscardOutcome::default()
            });
        }
        let mut outcome = DiscardOutcome::default();
        if policy != SubmoduleCheckoutPolicy::KeepCheckout {
            self.check_out_submodule_at(&row, &target, risk.as_ref(), &mut outcome)?;
        }
        Ok(outcome)
    }

    pub(super) fn submodule_pointer_snapshot_impl(
        &self,
    ) -> Result<Option<SubmodulePointerSnapshot>> {
        if !self.spec.workdir.join(".gitmodules").is_file() {
            return Ok(None);
        }
        let repo = self.reopen_repo()?;
        if repo.config_snapshot().boolean("submodule.recurse") == Some(true) {
            return Ok(None);
        }
        // Uncommitted work inside is checked after the operation, and only
        // for the submodules whose pointer moved: the operation in the parent
        // leaves it as it was.
        let in_sync: Vec<(PathBuf, CommitId)> = self
            .submodule_rows_impl()?
            .into_iter()
            .filter(|row| !row.nested && row.status == SubmoduleStatus::UpToDate)
            .map(|row| (row.path, row.recorded_head))
            .collect();
        Ok((!in_sync.is_empty()).then_some(SubmodulePointerSnapshot { in_sync }))
    }

    pub(super) fn follow_submodule_pointers_impl(
        &self,
        snapshot: &SubmodulePointerSnapshot,
    ) -> Result<Vec<SubmoduleCheckoutUpdate>> {
        let mut rows = self.submodule_rows_impl()?;
        let mut moved: Vec<&mut Submodule> = rows
            .iter_mut()
            .filter(|row| {
                !row.nested
                    && snapshot.in_sync.iter().any(|(path, before)| {
                        *path == row.path
                            && row.recorded_head != *before
                            && row.checked_out_head.as_ref() == Some(before)
                    })
            })
            .collect();
        if moved.is_empty() {
            return Ok(Vec::new());
        }
        fill_inner_changes(&self.spec.workdir, moved.iter_mut().map(|row| &mut **row))?;
        let mut outcome = DiscardOutcome::default();
        for row in moved {
            if row.has_inner_changes {
                continue;
            }
            let target = row.recorded_head.clone();
            // One submodule that cannot follow (its new commit was never
            // fetched, say) keeps its out-of-sync marker; the rest go on.
            let _ = self.check_out_submodule_at(row, &target, None, &mut outcome);
        }
        Ok(outcome.updated)
    }

    /// What checking `row` out at `target` could lose, if anything.
    fn checkout_risk(&self, row: &Submodule, target: &CommitId) -> Option<SubmoduleCheckoutRisk> {
        let checked_out = row.checked_out_head.clone()?;
        let moves = checked_out != *target;
        // A commit whose refs cannot be read counts as unreferenced, so the
        // guard keeps it on a branch rather than risk losing it.
        let unreferenced = moves
            && !commit_is_referenced(&self.spec.workdir.join(&row.path), &checked_out)
                .unwrap_or(false);
        (row.has_inner_changes || unreferenced).then(|| SubmoduleCheckoutRisk {
            path: row.path.clone(),
            checked_out: Some(checked_out),
            target: target.clone(),
            has_inner_changes: row.has_inner_changes,
            unreferenced,
        })
    }

    /// Keep whatever `risk` names, then detach the submodule at `target`.
    fn check_out_submodule_at(
        &self,
        row: &Submodule,
        target: &CommitId,
        risk: Option<&SubmoduleCheckoutRisk>,
        outcome: &mut DiscardOutcome,
    ) -> Result<()> {
        let submodule_workdir = self.spec.workdir.join(&row.path);
        let target_short = short_commit_id(target.as_ref());
        if let Some(risk) = risk {
            if risk.has_inner_changes {
                let mut stash = git_workdir_cmd_for(&submodule_workdir);
                let parent = self
                    .spec
                    .workdir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.spec.workdir.display().to_string());
                stash.arg("stash").arg("push").arg("-m").arg(format!(
                    "Before checking out {target_short} to match {parent}"
                ));
                run_git_simple(stash, "git stash push")?;
                outcome.stashed.push(row.path.clone());
            }
            if risk.unreferenced
                && let Some(checked_out) = risk.checked_out.as_ref()
            {
                let branch = format!(
                    "{}/kept-{}",
                    gitcomet_core::identity::current().executable_name(),
                    short_commit_id(checked_out.as_ref())
                );
                let mut keep = git_workdir_cmd_for(&submodule_workdir);
                keep.arg("branch")
                    .arg("--")
                    .arg(&branch)
                    .arg(checked_out.as_ref());
                run_git_simple(keep, "git branch")?;
                outcome.kept_on_branch.push((row.path.clone(), branch));
            }
        }
        if row.checked_out_head.as_ref() == Some(target) {
            return Ok(());
        }
        if commit_exists(&submodule_workdir, target) {
            let mut checkout = git_workdir_cmd_for(&submodule_workdir);
            checkout
                .arg("checkout")
                .arg("-q")
                .arg("--detach")
                .arg(target.as_ref())
                .arg("--");
            run_git_simple(checkout, "git checkout --detach")?;
        } else {
            // Not fetched yet: let Git fetch it the way `submodule update`
            // does, still without forcing over local changes.
            let mut update = self.git_workdir_cmd();
            update.arg("submodule").arg("update").arg("--checkout");
            push_literal_paths(&mut update, [crate::util::submodule_script_path(&row.path)]);
            run_git_simple(update, "git submodule update --checkout")?;
        }
        outcome.updated.push(SubmoduleCheckoutUpdate {
            path: row.path.clone(),
            commit: target.clone(),
        });
        Ok(())
    }
}

impl GixRepo {
    /// Commit the submodule at `path` as checked out now, leaving everything
    /// else that is staged where it is (`git commit --only`).
    pub(super) fn record_submodule_commit_impl(
        &self,
        path: &Path,
        message: &str,
        identity: Option<&gitcomet_core::commit_identity::CommitIdentity>,
    ) -> Result<CommitOperationOutcome> {
        let local_branch = self.current_branch_name_for_outcome()?;
        let pre_head = self.head_commit_id_for_outcome()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit").arg("--only");
        if let Some(identity) = identity {
            identity.apply(&mut cmd)?;
        }
        cmd.arg("-m").arg(message);
        push_literal_paths(&mut cmd, &[path.to_path_buf()]);
        run_git_simple(cmd, "git commit --only")?;
        let post_head = self.head_commit_id_for_outcome()?;
        Ok(CommitOperationOutcome {
            local_branch,
            pre_head,
            post_head,
        })
    }

    /// Re-record the submodule at `path` in the commit that recorded it,
    /// keeping its message. Refuses unless HEAD is still `expected_head`, so
    /// a commit made in between is never rewritten.
    pub(super) fn amend_submodule_record_impl(
        &self,
        path: &Path,
        expected_head: &CommitId,
        identity: Option<&gitcomet_core::commit_identity::CommitIdentity>,
    ) -> Result<CommitOperationOutcome> {
        let local_branch = self.current_branch_name_for_outcome()?;
        let pre_head = self.head_commit_id_for_outcome()?;
        if pre_head.as_ref() != Some(expected_head) {
            return Err(Error::new(ErrorKind::Backend(format!(
                "HEAD moved since {} was recorded; record the submodule again by hand.",
                path.display()
            ))));
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit")
            .arg("--amend")
            .arg("--no-edit")
            .arg("--only");
        if let Some(identity) = identity {
            identity.apply(&mut cmd)?;
        }
        push_literal_paths(&mut cmd, &[path.to_path_buf()]);
        run_git_simple(cmd, "git commit --amend --only")?;
        let post_head = self.head_commit_id_for_outcome()?;
        Ok(CommitOperationOutcome {
            local_branch,
            pre_head,
            post_head,
        })
    }

    /// Stash the uncommitted changes to tracked files in the submodule at
    /// `path`: the changes the leave-behind guard counted. Untracked files
    /// stay where they are.
    pub(super) fn stash_submodule_changes_impl(&self, path: &Path, message: &str) -> Result<()> {
        let workdir = self.spec.workdir.join(path);
        let mut stash = git_workdir_cmd_for(&workdir);
        stash.arg("stash").arg("push").arg("-m").arg(message);
        run_git_simple(stash, "git stash push")
    }

    /// Make `commit` fetchable from the submodule at `path`: push the local
    /// branch that holds it to that branch's upstream. The checked-out branch
    /// wins; a submodule is often detached at the commit after `submodule
    /// update`, so another branch that holds it is pushed instead.
    pub(super) fn push_submodule_with_output_impl(
        &self,
        path: &Path,
        commit: &CommitId,
    ) -> Result<CommandOutput> {
        let workdir = self.spec.workdir.join(path);
        let mut refs = git_workdir_cmd_for(&workdir);
        refs.arg("for-each-ref")
            .arg("--format=%(HEAD)%00%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)")
            .arg(format!("--contains={}", commit.as_ref()))
            .arg("refs/heads");
        let output = run_git_background_output(
            refs,
            "git for-each-ref --contains refs/heads",
            &CancellationToken::new(),
        )?;
        let branches = if output.status.success() {
            parse_branches_with_upstream(&output.stdout)
        } else {
            Vec::new()
        };
        let Some(branch) = branches
            .iter()
            .find(|branch| branch.current)
            .or_else(|| branches.first())
        else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "No branch with an upstream in {} holds {}. Put the commit on a branch there, set its upstream and push it.",
                path.display(),
                short_commit_id(commit.as_ref())
            ))));
        };
        let mut push = git_workdir_cmd_for(&workdir);
        push.arg("push")
            .arg(&branch.remote)
            .arg(format!("{}:{}", branch.refname, branch.remote_ref));
        crate::util::run_git_with_output(push, "git push")
    }

    /// First-level submodule commits that outgoing commits record but that no
    /// remote-tracking branch or tag in the submodule holds, so nobody could
    /// fetch them after a push. Submodules that are not loaded are left out.
    pub(super) fn unpushed_submodule_pointers_impl(&self) -> Result<Vec<SubmoduleCheckoutUpdate>> {
        if !self.spec.workdir.join(".gitmodules").is_file() {
            return Ok(Vec::new());
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--format=")
            .arg("--raw")
            .arg("--no-abbrev")
            .arg("-z")
            .arg("--no-renames")
            .arg("HEAD")
            .arg("--not")
            .arg("--remotes");
        let output = run_git_background_output(
            cmd,
            "git log --raw HEAD --not --remotes",
            &CancellationToken::new(),
        )?;
        if !output.status.success() {
            // An unborn branch has no outgoing commits to check.
            return Ok(Vec::new());
        }
        let mut newest: Vec<SubmoduleCheckoutUpdate> = Vec::new();
        for (path, commit) in parse_raw_gitlinks(&output.stdout) {
            if newest.iter().all(|seen| seen.path != path) {
                newest.push(SubmoduleCheckoutUpdate { path, commit });
            }
        }
        let mut unpushed = Vec::new();
        for pointer in newest {
            let workdir = self.spec.workdir.join(&pointer.path);
            if !workdir.join(".git").exists() || !commit_exists(&workdir, &pointer.commit) {
                continue;
            }
            if !commit_on_remote(&workdir, &pointer.commit)? {
                unpushed.push(pointer);
            }
        }
        Ok(unpushed)
    }
}

/// `(path, new commit)` for each gitlink a `git log --raw -z` adds or moves,
/// newest first.
fn parse_raw_gitlinks(stdout: &[u8]) -> Vec<(PathBuf, CommitId)> {
    let mut gitlinks = Vec::new();
    let mut fields = stdout.split(|byte| *byte == 0);
    while let Some(meta) = fields.next() {
        let meta = meta.trim_ascii_start();
        if meta.first() != Some(&b':') {
            continue;
        }
        let Some(path) = fields.next() else {
            break;
        };
        let parts: Vec<&[u8]> = meta[1..].split(|byte| *byte == b' ').collect();
        let [_, new_mode, _, new_id, status] = parts.as_slice() else {
            continue;
        };
        if *new_mode != b"160000" || status.first() == Some(&b'D') {
            continue;
        }
        let Ok(id) = std::str::from_utf8(new_id) else {
            continue;
        };
        let Ok(path) = path_buf_from_git_bytes(path, "git log --raw path") else {
            continue;
        };
        gitlinks.push((path, CommitId(id.into())));
    }
    gitlinks
}

/// Whether a remote-tracking branch or a tag in the submodule holds
/// `commit`. Tags count because a submodule is often pinned to a release tag
/// that no remote branch contains.
fn commit_on_remote(workdir: &Path, commit: &CommitId) -> Result<bool> {
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("for-each-ref")
        .arg("--count=1")
        .arg("--format=%(refname)")
        .arg(format!("--contains={}", commit.as_ref()))
        .arg("refs/remotes")
        .arg("refs/tags");
    let output = run_git_background_output(
        cmd,
        "git for-each-ref --contains refs/remotes refs/tags",
        &CancellationToken::new(),
    )?;
    Ok(output.status.success() && !output.stdout.trim_ascii().is_empty())
}

/// A local branch with an upstream, from `for-each-ref` with the format
/// `%(HEAD)%00%(refname)%00%(upstream:remotename)%00%(upstream:remoteref)`.
#[derive(Debug, Eq, PartialEq)]
struct BranchWithUpstream {
    current: bool,
    refname: String,
    remote: String,
    remote_ref: String,
}

fn parse_branches_with_upstream(stdout: &[u8]) -> Vec<BranchWithUpstream> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let current = fields.next()? == "*";
            let refname = fields.next()?;
            let remote = fields.next()?;
            let remote_ref = fields.next()?;
            (!remote.is_empty() && !remote_ref.is_empty()).then(|| BranchWithUpstream {
                current,
                refname: refname.to_string(),
                remote: remote.to_string(),
                remote_ref: remote_ref.to_string(),
            })
        })
        .collect()
}

fn commit_exists(workdir: &Path, commit: &CommitId) -> bool {
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("cat-file")
        .arg("-e")
        .arg(format!("{}^{{commit}}", commit.as_ref()));
    run_git_simple(cmd, "git cat-file -e").is_ok()
}

/// Whether a branch, remote-tracking branch or tag holds `commit`.
fn commit_is_referenced(workdir: &Path, commit: &CommitId) -> Result<bool> {
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("for-each-ref")
        .arg("--count=1")
        .arg("--format=%(refname)")
        .arg(format!("--contains={}", commit.as_ref()))
        .arg("refs/heads")
        .arg("refs/remotes")
        .arg("refs/tags");
    let output = run_git_background_output(
        cmd,
        "git for-each-ref --contains",
        &CancellationToken::new(),
    )?;
    if !output.status.success() {
        return Err(Error::new(ErrorKind::Backend(
            "could not read which refs hold the submodule commit".into(),
        )));
    }
    Ok(!output.stdout.trim_ascii().is_empty())
}

/// The working tree of the repository that records `workdir` as a gitlink,
/// canonicalized. Git is asked only when an ancestor directory has a
/// `.gitmodules`, so opening an ordinary repository costs no process.
fn superproject_working_tree(workdir: &Path) -> Result<Option<PathBuf>> {
    if !workdir
        .ancestors()
        .skip(1)
        .any(|dir| dir.join(".gitmodules").is_file())
    {
        return Ok(None);
    }
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("rev-parse").arg("--show-superproject-working-tree");
    let output = run_git_background_output(
        cmd,
        "git rev-parse --show-superproject-working-tree",
        &CancellationToken::new(),
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = output.stdout.trim_ascii();
    if stdout.is_empty() {
        return Ok(None);
    }
    let parent = path_buf_from_git_bytes(stdout, "superproject working tree")?;
    Ok(Some(canonicalize_or_original(parent)))
}

/// Fill drift and inner-change facts for the first-level rows of a listing of
/// the repository checked out at `workdir`. Facts that cannot be read leave
/// the row as it was: they only refine the pill the sidebar already shows.
pub(super) fn fill_first_level_sync_facts(
    workdir: &Path,
    rows: &mut [Submodule],
    cancellation: &CancellationToken,
) -> Result<()> {
    fill_inner_changes_cancellable(workdir, rows.iter_mut(), cancellation)?;
    for row in rows
        .iter_mut()
        .filter(|row| !row.nested && row.status == SubmoduleStatus::HeadMismatch)
    {
        cancellation.check_cancelled()?;
        let Some(checked_out) = row.checked_out_head.as_ref() else {
            continue;
        };
        row.drift = commit_drift(
            &workdir.join(&row.path),
            row.recorded_head.as_ref(),
            checked_out.as_ref(),
            cancellation,
        )?;
    }
    Ok(())
}

/// Set `has_inner_changes` on the loaded first-level rows among `rows`.
fn fill_inner_changes<'a>(
    workdir: &Path,
    rows: impl IntoIterator<Item = &'a mut Submodule>,
) -> Result<()> {
    fill_inner_changes_cancellable(workdir, rows, &CancellationToken::new())
}

fn fill_inner_changes_cancellable<'a>(
    workdir: &Path,
    rows: impl IntoIterator<Item = &'a mut Submodule>,
    cancellation: &CancellationToken,
) -> Result<()> {
    let mut loaded: Vec<&mut Submodule> = rows
        .into_iter()
        .filter(|row| !row.nested && row.checked_out_head.is_some())
        .collect();
    if loaded.is_empty() {
        return Ok(());
    }
    let paths: Vec<PathBuf> = loaded.iter().map(|row| row.path.clone()).collect();
    for (path, inner) in inner_change_flags(workdir, &paths, cancellation)? {
        if let Some(row) = loaded.iter_mut().find(|row| row.path == path) {
            row.has_inner_changes = inner;
        }
    }
    Ok(())
}

/// Which of `paths` have uncommitted changes to tracked files inside, per the
/// superproject's `git status` submodule field (`S.M.`). Untracked files are
/// left out, as a commit or stash would leave them, and the repository's own
/// `submodule.<name>.ignore` settings apply.
fn inner_change_flags(
    workdir: &Path,
    paths: &[PathBuf],
    cancellation: &CancellationToken,
) -> Result<Vec<(PathBuf, bool)>> {
    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("-z")
        .arg("--untracked-files=no");
    push_literal_paths(&mut cmd, paths);
    let output = run_git_background_output(cmd, "git status --porcelain=v2", cancellation)?;
    if !output.status.success() {
        // The checkout guard must not read "no changes" from a failed status.
        return Err(Error::new(ErrorKind::Backend(format!(
            "could not read the submodules' status: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))));
    }
    let mut flags = Vec::new();
    let mut records = output.stdout.split(|byte| *byte == 0).peekable();
    while let Some(record) = records.next() {
        match record.first() {
            Some(b'1') => {
                if let Some(parsed) = parse_submodule_record(record, 8) {
                    flags.push(parsed);
                }
            }
            Some(b'2') => {
                if let Some(parsed) = parse_submodule_record(record, 9) {
                    flags.push(parsed);
                }
                let _ = records.next();
            }
            _ => {}
        }
    }
    Ok(flags)
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`; a rename record has one
/// more field before the path. Returns the path and whether the submodule
/// field reports tracked or untracked changes inside it.
fn parse_submodule_record(record: &[u8], path_field: usize) -> Option<(PathBuf, bool)> {
    let mut fields = record.splitn(path_field + 1, |byte| *byte == b' ');
    let _kind = fields.next()?;
    let _xy = fields.next()?;
    let sub = fields.next()?;
    if sub.first() != Some(&b'S') || sub.len() != 4 {
        return None;
    }
    let path = fields.nth(path_field - 3)?;
    let inner = sub[2] == b'M' || sub[3] == b'U';
    path_buf_from_git_bytes(path, "submodule status path")
        .ok()
        .map(|path| (path, inner))
}

/// Commits `checked_out` has that `recorded` lacks, and the reverse, counted
/// in the submodule. `None` when either commit is missing there.
fn commit_drift(
    submodule_workdir: &Path,
    recorded: &str,
    checked_out: &str,
    cancellation: &CancellationToken,
) -> Result<Option<SubmoduleDrift>> {
    if recorded == checked_out {
        return Ok(Some(SubmoduleDrift::default()));
    }
    let mut cmd = git_workdir_cmd_for(submodule_workdir);
    cmd.arg("rev-list")
        .arg("--left-right")
        .arg("--count")
        .arg(format!("{checked_out}...{recorded}"))
        .arg("--");
    let output = run_git_background_output(cmd, "git rev-list --left-right --count", cancellation)?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse_left_right_counts(&output.stdout))
}

fn parse_left_right_counts(stdout: &[u8]) -> Option<SubmoduleDrift> {
    let text = std::str::from_utf8(stdout).ok()?;
    let mut counts = text.split_ascii_whitespace();
    let ahead = counts.next()?.parse().ok()?;
    let behind = counts.next()?.parse().ok()?;
    Some(SubmoduleDrift { ahead, behind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_without_an_upstream_cannot_be_pushed_to_one() {
        let stdout = b"*\0refs/heads/main\0\0\n \0refs/heads/topic\0origin\0refs/heads/topic\n";
        assert_eq!(
            parse_branches_with_upstream(stdout),
            vec![BranchWithUpstream {
                current: false,
                refname: "refs/heads/topic".into(),
                remote: "origin".into(),
                remote_ref: "refs/heads/topic".into(),
            }]
        );
    }

    #[test]
    fn raw_gitlinks_keep_added_and_moved_pointers_only() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let z = "0".repeat(40);
        let raw = format!(
            ":160000 160000 {a} {b} M\0mods/child\0:100644 100644 {a} {b} M\0README\0:000000 160000 {z} {a} A\0libs/new\0:160000 000000 {a} {z} D\0libs/gone\0"
        );
        assert_eq!(
            parse_raw_gitlinks(raw.as_bytes()),
            vec![
                (PathBuf::from("mods/child"), CommitId(b.as_str().into())),
                (PathBuf::from("libs/new"), CommitId(a.as_str().into())),
            ]
        );
    }

    #[test]
    fn submodule_records_report_tracked_and_untracked_changes() {
        let modified = b"1 .M SCM. 160000 160000 160000 aaaa bbbb fixtures/syntax test";
        assert_eq!(
            parse_submodule_record(modified, 8),
            Some((PathBuf::from("fixtures/syntax test"), true))
        );
        let untracked = b"1 .M S..U 160000 160000 160000 aaaa bbbb vendor/lib";
        assert_eq!(
            parse_submodule_record(untracked, 8),
            Some((PathBuf::from("vendor/lib"), true))
        );
        let moved_only = b"1 .M SC.. 160000 160000 160000 aaaa bbbb vendor/lib";
        assert_eq!(
            parse_submodule_record(moved_only, 8),
            Some((PathBuf::from("vendor/lib"), false))
        );
        let plain_file = b"1 .M N... 100644 100644 100644 aaaa bbbb src/main.rs";
        assert_eq!(parse_submodule_record(plain_file, 8), None);
    }

    #[test]
    fn left_right_counts_read_ahead_then_behind() {
        assert_eq!(
            parse_left_right_counts(b"2\t1\n"),
            Some(SubmoduleDrift {
                ahead: 2,
                behind: 1
            })
        );
        assert_eq!(parse_left_right_counts(b"garbage"), None);
    }
}
