//! Git LFS operations, run through `git lfs` so hooks, credentials and the
//! user's transfer configuration all apply.

use crate::util::{
    run_git_background_capture, run_git_with_output, run_git_with_output_until_done,
    validate_ref_like_arg,
};
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::git_operation::{self, GitOperationEvent};
use gitcomet_core::large_files::{LargeFileCommand, LfsLock, lfs_include_pattern};
use gitcomet_core::lfs;
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::path::PathBuf;

fn paths_arg_error(what: &str) -> Error {
    Error::new(ErrorKind::Backend(format!("{what}: no paths given")))
}

/// Join the outputs of a multi-step command into one log entry.
pub(super) fn combine(label: &str, outputs: Vec<CommandOutput>) -> CommandOutput {
    let join = |pick: fn(&CommandOutput) -> &str| {
        outputs
            .iter()
            .map(pick)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    CommandOutput {
        command: label.to_string(),
        stdout: join(|output| &output.stdout),
        stderr: join(|output| &output.stderr),
        exit_code: outputs.last().and_then(|output| output.exit_code),
    }
}

impl super::GixRepo {
    fn git_lfs(&self, args: &[&str]) -> std::process::Command {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").args(args);
        cmd
    }

    pub(super) fn run_large_file_command_impl(
        &self,
        command: &LargeFileCommand,
    ) -> Result<CommandOutput> {
        // A download may fetch several batches, and locks may involve more
        // than one command. Every step of a retry needs the same credentials.
        if command.is_annex() {
            // Annex scopes its own credentials, excluding failure cleanup.
            self.run_large_file_command_inner(command)
        } else {
            crate::util::with_shared_git_auth(|| self.run_large_file_command_inner(command))
        }
    }

    fn run_large_file_command_inner(&self, command: &LargeFileCommand) -> Result<CommandOutput> {
        // Explicit LFS/annex commands need a Cancel control even when they
        // produce no hook events, output or transfer progress.
        if let Some(operation) = git_operation::current() {
            operation.emit(GitOperationEvent::CommandStarted);
        }
        match command {
            LargeFileCommand::LfsPull { paths } => self.lfs_pull(paths),
            LargeFileCommand::LfsFetchForDiff { target } => self.lfs_fetch_for_diff(target),
            LargeFileCommand::LfsFetchAll => {
                run_git_with_output(self.git_lfs(&["fetch", "--all"]), "git lfs fetch --all")
            }
            LargeFileCommand::LfsPushAll { remote } => {
                validate_ref_like_arg(remote, "remote")?;
                let mut cmd = self.git_lfs(&["push", "--all"]);
                cmd.arg(remote);
                run_git_with_output(cmd, "git lfs push --all")
            }
            LargeFileCommand::LfsPrune => {
                run_git_with_output_until_done(self.git_lfs(&["prune"]), "git lfs prune")
            }
            LargeFileCommand::LfsFsck => {
                run_git_with_output_until_done(self.git_lfs(&["fsck"]), "git lfs fsck")
            }
            LargeFileCommand::LfsInstall => run_git_with_output(
                self.git_lfs(&["install", "--local"]),
                "git lfs install --local",
            ),
            LargeFileCommand::LfsLock { paths } => {
                self.lfs_paths_command(&["lock"], paths, "git lfs lock")
            }
            LargeFileCommand::LfsUnlock { paths, force } => {
                let args: &[&str] = if *force {
                    &["unlock", "--force"]
                } else {
                    &["unlock"]
                };
                self.lfs_paths_command(args, paths, "git lfs unlock")
            }
            LargeFileCommand::LfsTrack {
                patterns,
                filename,
                lockable,
                renormalize,
            } => self.lfs_track(patterns, *filename, *lockable, renormalize),
            annex => self.run_annex_command(annex),
        }
    }

    /// `git lfs pull` has no pathspec, so fetch exactly these objects, then
    /// check out just these paths.
    fn lfs_pull(&self, paths: &[PathBuf]) -> Result<CommandOutput> {
        if paths.is_empty() {
            return run_git_with_output(self.git_lfs(&["pull"]), "git lfs pull");
        }
        let patterns = paths
            .iter()
            .map(|path| {
                let relative = path.strip_prefix(&self.spec.workdir).unwrap_or(path);
                lfs_include_pattern(relative).ok_or_else(|| {
                    Error::new(ErrorKind::Backend(format!(
                        "cannot select `{}` with a Git LFS include pattern",
                        path.display()
                    )))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut revisions = self.lfs_local_trees(paths, false)?;
        if let Ok(Some(head)) = crate::refs::head_oid(&self.repo()) {
            revisions.push(head.to_string());
        }
        let fetched = self.lfs_fetch_revisions(&patterns, revisions)?;
        let mut checkout = self.git_lfs(&["checkout", "--"]);
        // Checkout also accepts gitignore globs, not literal pathspecs.
        checkout.args(&patterns);
        let checked_out = run_git_with_output(checkout, "git lfs checkout")?;
        Ok(combine("git lfs pull", vec![fetched, checked_out]))
    }

    fn lfs_fetch_for_diff(&self, target: &DiffTarget) -> Result<CommandOutput> {
        let mut local_trees = Vec::new();
        let (path, mut revisions) = match target {
            DiffTarget::WorkingTree { path, area, .. } => {
                // The index side need not be in HEAD (after `reset --soft`, or
                // theirs in a merge), and git-lfs fetches by commit or tree.
                // An unstaged worktree pointer can also differ from both.
                local_trees =
                    self.lfs_local_trees(std::slice::from_ref(path), *area == DiffArea::Unstaged)?;
                let head = crate::refs::head_oid(&self.repo()).ok().flatten().is_some();
                (path, head.then(|| "HEAD".to_string()).into_iter().collect())
            }
            DiffTarget::Commit {
                commit_id, path, ..
            } => {
                let mut revisions = vec![commit_id.as_ref().to_string()];
                if let Some(parent) =
                    super::diff::gix_first_parent_optional(&self.repo(), commit_id.as_ref())?
                {
                    revisions.push(parent);
                }
                (path, revisions)
            }
            DiffTarget::CommitRange {
                from_commit_id,
                to_commit_id,
                path: Some(path),
                ..
            } => {
                let mut revisions = vec![from_commit_id.as_ref().to_string()];
                if let Some(to) = to_commit_id {
                    revisions.push(to.as_ref().to_string());
                } else {
                    local_trees = self.lfs_local_trees(std::slice::from_ref(path), true)?;
                }
                (path, revisions)
            }
            _ => return Err(paths_arg_error("git lfs fetch for diff")),
        };
        // Resolve refs before passing them as positional arguments.
        let repo = self.repo();
        for revision in &mut revisions {
            validate_ref_like_arg(revision, "revision")?;
            *revision = crate::refs::resolve_required(&repo, revision)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("resolve LFS revision: {e}"))))?
                .detach()
                .to_string();
        }
        revisions.extend(local_trees);
        let relative = path.strip_prefix(&self.spec.workdir).unwrap_or(path);
        let include = lfs_include_pattern(relative).ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "Git LFS cannot select this path with an include pattern".to_string(),
            ))
        })?;
        let mut patterns = vec![include];
        // A rename's old side lives at the source path in the older revision.
        if let Some(old_path) = target.old_file_path() {
            let relative = old_path
                .strip_prefix(&self.spec.workdir)
                .unwrap_or(old_path);
            patterns.extend(lfs_include_pattern(relative));
        }
        self.lfs_fetch_revisions(&patterns, revisions)
    }

    fn lfs_fetch_revisions(
        &self,
        patterns: &[String],
        mut revisions: Vec<String>,
    ) -> Result<CommandOutput> {
        revisions.sort_unstable();
        revisions.dedup();
        if revisions.is_empty() {
            return Err(paths_arg_error("git lfs fetch"));
        }
        // Positional refs work on git-lfs versions before 3.8 (--stdin does
        // not). Match git-lfs's default download-remote precedence.
        let repo = self.repo_with_current_config()?;
        let config = repo.config_snapshot();
        let branch_remote = crate::refs::head_name(&repo)
            .ok()
            .flatten()
            .and_then(|head| {
                config
                    .string(&format!("branch.{}.remote", head.shorten()))
                    .map(|value| value.to_string())
            });
        let remote = branch_remote
            .or_else(|| {
                config
                    .string("remote.lfsdefault")
                    .map(|value| value.to_string())
            })
            .or_else(|| {
                let remotes = repo.remote_names();
                (remotes.len() == 1).then(|| remotes.iter().next().unwrap().to_string())
            })
            .unwrap_or_else(|| "origin".into());
        validate_ref_like_arg(&remote, "remote")?;
        let mut outputs = Vec::new();
        // Bound the argument list on platforms with small command-line limits.
        for refs in revisions.chunks(128) {
            let mut cmd = self.git_lfs(&["fetch", "--exclude="]);
            cmd.arg(format!("--include={}", patterns.join(",")));
            cmd.arg("--").arg(&remote).args(refs);
            outputs.push(run_git_with_output(cmd, "git lfs fetch")?);
        }
        Ok(combine("git lfs fetch", outputs))
    }

    /// One tree per index stage and, when displayed, the worktree pointer, so
    /// git-lfs can fetch content absent from HEAD. Only loose objects are
    /// written; the index and worktree are untouched.
    fn lfs_local_trees(&self, paths: &[PathBuf], include_worktree: bool) -> Result<Vec<String>> {
        use gix::objs::tree::{Entry, EntryKind};
        let repo = self.repo();
        let backend = |e: &dyn std::fmt::Display| {
            Error::new(ErrorKind::Backend(format!("local side for LFS fetch: {e}")))
        };
        let index = repo.index_or_empty().map_err(|e| backend(&e))?;
        let mut trees = Vec::new();
        for path in paths {
            let relative = path.strip_prefix(&self.spec.workdir).unwrap_or(path);
            let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative));
            let mut blobs: Vec<_> = index
                .entry_range(key.as_ref())
                .into_iter()
                .flat_map(|range| &index.entries()[range])
                .map(|entry| entry.id)
                .collect();
            let full = self.spec.workdir.join(relative);
            if include_worktree
                && std::fs::symlink_metadata(&full).is_ok_and(|metadata| metadata.is_file())
                && let Some(bytes) = super::large_files::read_pointer_candidate(&full)
                && lfs::parse_pointer(&bytes).is_some()
            {
                blobs.push(repo.write_blob(&bytes).map_err(|e| backend(&e))?.detach());
            }
            for mut id in blobs {
                let mut kind = EntryKind::Blob;
                for name in key.split(|byte| *byte == b'/').rev() {
                    id = repo
                        .write_object(gix::objs::Tree {
                            entries: vec![Entry {
                                mode: kind.into(),
                                filename: name.into(),
                                oid: id,
                            }],
                        })
                        .map_err(|e| backend(&e))?
                        .detach();
                    kind = EntryKind::Tree;
                }
                trees.push(id.to_string());
            }
        }
        Ok(trees)
    }

    fn lfs_paths_command(
        &self,
        args: &[&str],
        paths: &[PathBuf],
        label: &str,
    ) -> Result<CommandOutput> {
        if paths.is_empty() {
            return Err(paths_arg_error(label));
        }
        let mut cmd = self.git_lfs(args);
        cmd.arg("--").args(paths);
        run_git_with_output(cmd, label)
    }

    fn lfs_track(
        &self,
        patterns: &[String],
        filename: bool,
        lockable: bool,
        renormalize: &[PathBuf],
    ) -> Result<CommandOutput> {
        if patterns.is_empty() {
            return Err(paths_arg_error("git lfs track"));
        }
        let mut track = self.git_lfs(&["track"]);
        if filename {
            track.arg("--filename");
        }
        if lockable {
            track.arg("--lockable");
        }
        track.arg("--").args(patterns);
        let mut outputs = vec![run_git_with_output(track, "git lfs track")?];
        if !renormalize.is_empty() {
            // The new attributes must be staged with the files they convert.
            // --renormalize implies -u and cannot add a new .gitattributes.
            let mut attributes = self.git_workdir_cmd();
            attributes.args(["add", "--", ".gitattributes"]);
            // Updating the index can refresh other tracked files too, invoking
            // their clean filters before the explicit renormalization below.
            outputs.push(run_git_with_output_until_done(
                attributes,
                "git add .gitattributes",
            )?);
            let mut add = self.git_workdir_cmd();
            add.args(["--literal-pathspecs", "add", "--renormalize", "--"])
                .args(renormalize);
            outputs.push(run_git_with_output_until_done(
                add,
                "git add --renormalize",
            )?);
        }
        Ok(combine("git lfs track", outputs))
    }

    /// Loads on its own when lockable patterns exist, so it runs as a
    /// background read: stored credentials may answer, but nothing prompts.
    pub(super) fn lfs_locks_impl(&self, cancellation: &CancellationToken) -> Result<Vec<LfsLock>> {
        let mut cmd = self.git_workdir_cmd();
        crate::util::credentials_without_prompts(&mut cmd);
        cmd.args(["lfs", "locks", "--json"]);
        let json = run_git_background_capture(cmd, "git lfs locks --json", cancellation)?;
        parse_lfs_locks_json(&json)
    }
}

/// `git lfs locks --json`: `[{"id","path","owner":{"name"},"locked_at"}]`.
pub(super) fn parse_lfs_locks_json(json: &str) -> Result<Vec<LfsLock>> {
    let value: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("git lfs locks --json: {e}"))))?;
    let entries = value.as_array().cloned().unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_string);
            Some(LfsLock {
                id: text("id")?,
                path: PathBuf::from(text("path")?),
                owner: entry
                    .get("owner")
                    .and_then(|owner| owner.get("name"))
                    .and_then(|name| name.as_str())
                    .map(str::to_string),
                locked_at: text("locked_at"),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lock_listings() {
        let json = r#"[{"id":"1","path":"art/hero.psd","owner":{"name":"alice"},"locked_at":"2026-09-20T10:00:00Z"},
                       {"id":"2","path":"b.bin"},{"path":"missing-id"}]"#;
        let locks = parse_lfs_locks_json(json).unwrap();
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].owner.as_deref(), Some("alice"));
        assert_eq!(locks[1].owner, None);
        assert!(parse_lfs_locks_json("[]").unwrap().is_empty());
        assert!(parse_lfs_locks_json("not json").is_err());
    }
}
