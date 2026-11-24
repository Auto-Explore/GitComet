use super::GixRepo;
use crate::util::{
    path_buf_from_git_bytes, run_git_capture_bytes_cancellable, run_git_with_output,
    validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Worktree};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CancellationToken, CommandOutput, Result, WorktreeCheckout};
use gix::bstr::ByteSlice as _;
use std::path::Path;
use std::process::Command;

mod native_config;
#[cfg(test)]
mod native_config_tests;

impl GixRepo {
    pub(super) fn list_worktrees_impl(&self) -> Result<Vec<Worktree>> {
        self.list_worktrees_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn list_worktrees_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Worktree>> {
        cancellation.check_cancelled()?;
        // The existing layout can only reject the optimization. In particular,
        // linked checkouts should not reparse configuration before invoking Git.
        // A positive still refreshes refs/config and rechecks the whole layout;
        // registrations and configuration can change while this handle is open.
        if !worktree_environment_overridden()
            && self.may_be_sole_primary_worktree()
            && let Some(repo) = native_config::reopen(self)
        {
            cancellation.check_cancelled()?;
            if let Some(worktree) = sole_primary_worktree(&repo) {
                cancellation.check_cancelled()?;
                return Ok(vec![worktree]);
            }
        }
        cancellation.check_cancelled()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("list").arg("--porcelain").arg("-z");
        let output = run_git_capture_bytes_cancellable(
            cmd,
            "git worktree list --porcelain -z",
            cancellation,
        )?;
        parse_git_worktree_list_porcelain_z(&output)
    }

    /// A stale negative only forgoes the native path; a positive is not evidence
    /// that a listing can be returned without the fresh authoritative checks.
    fn may_be_sole_primary_worktree(&self) -> bool {
        let common_dir = {
            let store = self.store.read().expect("repo store");
            let git_dir = store.repo.git_dir();
            let common_dir = store.repo.common_dir.as_deref().unwrap_or(git_dir);
            if git_dir != common_dir {
                return false;
            }
            common_dir.to_path_buf()
        };
        // The store guard must not outlive the path read: filesystem I/O and
        // the subsequent fresh open must not delay a store replacement.
        no_worktree_registrations(&common_dir)
    }

    pub(super) fn add_worktree_with_output_impl(
        &self,
        path: &Path,
        checkout: &WorktreeCheckout,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = match checkout {
            WorktreeCheckout::Existing { reference } => {
                push_worktree_args(&mut cmd, &["add"], path, Some(reference))?
            }
            WorktreeCheckout::NewBranch { name, start } => {
                validate_ref_like_arg(name, "worktree branch")?;
                push_worktree_args(
                    &mut cmd,
                    &["add", "--no-track", "-b", name],
                    path,
                    start.as_deref(),
                )?
            }
            WorktreeCheckout::TrackRemote { name, upstream } => {
                validate_ref_like_arg(name, "worktree branch")?;
                validate_ref_like_arg(&upstream.remote, "remote name")?;
                validate_ref_like_arg(&upstream.branch, "branch name")?;
                let tracking_ref = self.live_tracking_ref(upstream, "git worktree add --track")?;
                push_worktree_args(
                    &mut cmd,
                    &["add", "--track", "-b", name],
                    path,
                    Some(&tracking_ref),
                )?
            }
        };
        run_git_with_output(cmd, &label)
    }

    pub(super) fn remove_worktree_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        // The refusals `--force` overrides are recognized by Git's English text.
        cmd.env("LC_ALL", "C");
        let label = push_worktree_args(&mut cmd, &["remove"], path, None)?;
        run_git_with_output(cmd, &label)
    }

    pub(super) fn force_remove_worktree_with_output_impl(
        &self,
        path: &Path,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["remove", "--force"], path, None)?;
        run_git_with_output(cmd, &label)
    }
}

fn worktree_environment_overridden() -> bool {
    [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

fn no_worktree_registrations(common_dir: &Path) -> bool {
    let registrations = common_dir.join("worktrees");
    match std::fs::symlink_metadata(&registrations) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Ok(metadata) if plain_directory(&metadata) => {
            // An entry or a failed enumeration may represent another checkout.
            std::fs::read_dir(&registrations).is_ok_and(|mut entries| entries.next().is_none())
        }
        _ => false,
    }
}

/// Only the conventional primary checkout with a resolved HEAD is handled
/// here. Git remains authoritative for linked, bare and unusual layouts.
fn sole_primary_worktree(repo: &gix::Repository) -> Option<Worktree> {
    if repo.git_dir_trust() != gix::sec::Trust::Full
        || repo.is_bare()
        || repo.git_dir() != repo.common_dir()
        || crate::refs::backend(repo).ok()? != crate::refs::RefBackend::Files
        || repo.config_snapshot().string("core.worktree").is_some()
    {
        return None;
    }
    let workdir = repo.workdir()?;
    let dot_git = workdir.join(".git");
    if !plain_directory(&std::fs::symlink_metadata(workdir).ok()?)
        || !plain_directory(&std::fs::symlink_metadata(&dot_git).ok()?)
        || !same_file::is_same_file(repo.git_dir(), &dot_git).ok()?
    {
        return None;
    }

    if !no_worktree_registrations(repo.common_dir()) {
        return None;
    }

    let refs = crate::refs::view(repo).ok()?;
    let (id, branch, detached) = match refs.find_exact(b"HEAD").ok()??.target {
        gix::refs::Target::Symbolic(name) => {
            let branch = name.as_bstr().strip_prefix(b"refs/heads/")?;
            // Symbolic chains and unborn refs retain Git's existing behavior.
            let gix::refs::Target::Object(id) = refs.find_exact(name.as_bstr()).ok()??.target
            else {
                return None;
            };
            (id, Some(branch.to_str_lossy().into_owned()), false)
        }
        gix::refs::Target::Object(id) => (id, None, true),
    };
    // Like Git's porcelain listing, return the ref's OID without loading its
    // object. Missing objects and detached tag targets still have a worktree row.
    let mut worktree = Worktree {
        path: workdir.to_path_buf(),
        bare: false,
        head: Some(CommitId(super::oid_to_arc_str(&id))),
        branch,
        detached,
    };
    canonicalize_worktree_path(&mut worktree);
    Some(worktree)
}

fn plain_directory(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        // Junctions and other reparse points are not all exposed as symlinks.
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_dir() && !metadata.file_type().is_symlink()
}

/// Append `git worktree <args> -- <path> [<reference>]` and return the display
/// label.
///
/// `path` and `reference` are typed by the user, so `--` keeps a value that
/// starts with `-` out of the option parser, and the reference is checked the
/// way every other ref argument in this crate is.
fn push_worktree_args(
    cmd: &mut Command,
    args: &[&str],
    path: &Path,
    reference: Option<&str>,
) -> Result<String> {
    if let Some(reference) = reference {
        validate_ref_like_arg(reference, "worktree reference")?;
    }
    // The label is a human-readable summary, so `--` stays an argv concern only.
    cmd.arg("worktree").args(args).arg("--").arg(path);
    let mut label = format!("git worktree {} {}", args.join(" "), path.display());
    if let Some(reference) = reference {
        cmd.arg(reference);
        label.push(' ');
        label.push_str(reference);
    }
    Ok(label)
}

pub(super) fn parse_git_worktree_list_porcelain_z(output: &[u8]) -> Result<Vec<Worktree>> {
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;

    for field in output.split(|b| *b == b'\0') {
        if field.is_empty() {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            continue;
        }

        if let Some(rest) = field.strip_prefix(b"worktree ") {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            current = Some(Worktree {
                path: path_buf_from_git_bytes(rest, "git worktree list path")?,
                bare: false,
                head: None,
                branch: None,
                detached: false,
            });
            continue;
        }

        let Some(wt) = current.as_mut() else {
            continue;
        };

        if let Some(rest) = field.strip_prefix(b"HEAD ") {
            if !rest.is_empty() {
                wt.head = Some(CommitId(String::from_utf8_lossy(rest).into_owned().into()));
            }
        } else if let Some(rest) = field.strip_prefix(b"branch ") {
            let branch = String::from_utf8_lossy(rest);
            if let Some(stripped) = branch.strip_prefix("refs/heads/") {
                wt.branch = Some(stripped.to_string());
            } else if !branch.is_empty() {
                wt.branch = Some(branch.into_owned());
            }
        } else if field == b"bare" {
            wt.bare = true;
        } else if field == b"detached" {
            wt.detached = true;
            wt.branch = None;
        }
    }

    if let Some(mut wt) = current.take() {
        canonicalize_worktree_path(&mut wt);
        out.push(wt);
    }

    Ok(out)
}

fn canonicalize_worktree_path(worktree: &mut Worktree) {
    worktree.path = canonicalize_or_original(worktree.path.clone());
}

#[cfg(test)]
mod tests;
