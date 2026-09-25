//! Git LFS operations, run through `git lfs` so hooks, credentials and the
//! user's transfer configuration all apply.

use crate::util::{
    run_git_background_capture, run_git_with_input_output, run_git_with_output,
    validate_ref_like_arg,
};
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::large_files::{LargeFileCommand, LfsLock, lfs_include_pattern};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::path::PathBuf;

fn paths_arg_error(what: &str) -> Error {
    Error::new(ErrorKind::Backend(format!("{what}: no paths given")))
}

/// Join the outputs of a multi-step command into one log entry.
fn combine(label: &str, outputs: Vec<CommandOutput>) -> CommandOutput {
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
                run_git_with_output(self.git_lfs(&["prune"]), "git lfs prune")
            }
            LargeFileCommand::LfsFsck => {
                run_git_with_output(self.git_lfs(&["fsck"]), "git lfs fsck")
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
                lfs_include_pattern(path).ok_or_else(|| {
                    Error::new(ErrorKind::Backend(format!(
                        "cannot download `{}` by name: Git LFS patterns cannot contain commas",
                        path.display()
                    )))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let include = patterns.join(",");
        let mut fetch = self.git_lfs(&["fetch"]);
        fetch.arg(format!("--include={include}")).arg("--exclude=");
        let fetched = run_git_with_output(fetch, "git lfs fetch")?;
        let mut checkout = self.git_lfs(&["checkout", "--"]);
        // Checkout also accepts gitignore globs, not literal pathspecs.
        checkout.args(&patterns);
        let checked_out = run_git_with_output(checkout, "git lfs checkout")?;
        Ok(combine("git lfs pull", vec![fetched, checked_out]))
    }

    fn lfs_fetch_for_diff(&self, target: &DiffTarget) -> Result<CommandOutput> {
        let mut index_trees = Vec::new();
        let (path, mut revisions) = match target {
            DiffTarget::WorkingTree { path, .. } => {
                // The index side need not be in HEAD (after `reset --soft`, or
                // theirs in a merge), and git-lfs fetches by commit or tree.
                index_trees = self.index_side_trees(path)?;
                let head = self.repo().head_id().is_ok();
                (path, head.then(|| "HEAD".to_string()).into_iter().collect())
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
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
            } => (
                path,
                vec![
                    from_commit_id.as_ref().to_string(),
                    to_commit_id
                        .as_ref()
                        .map_or("HEAD", |id| id.as_ref())
                        .to_string(),
                ],
            ),
            _ => return Err(paths_arg_error("git lfs fetch for diff")),
        };
        // Resolve to object ids before writing the line-delimited stdin protocol.
        // Git LFS chooses the remote itself, just as for its ordinary fetch.
        for revision in &mut revisions {
            validate_ref_like_arg(revision, "revision")?;
            *revision = self
                .repo()
                .rev_parse_single(revision.as_str())
                .map_err(|e| Error::new(ErrorKind::Backend(format!("resolve LFS revision: {e}"))))?
                .detach()
                .to_string();
        }
        revisions.extend(index_trees);
        revisions.sort();
        revisions.dedup();
        if revisions.is_empty() {
            return Err(paths_arg_error("git lfs fetch for diff"));
        }
        let relative = path.strip_prefix(&self.spec.workdir).unwrap_or(path);
        let include = lfs_include_pattern(relative).ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "Git LFS cannot select this path with an include pattern".to_string(),
            ))
        })?;
        let mut cmd = self.git_lfs(&["fetch", "--stdin", "--exclude="]);
        cmd.arg(format!("--include={include}"));
        run_git_with_input_output(
            cmd,
            "git lfs fetch",
            format!("{}\n", revisions.join("\n")).as_bytes(),
        )
    }

    /// One tree per index stage of `path`, holding just that blob at that
    /// path, so git-lfs can fetch the index side. Only loose objects are
    /// written; the index itself is untouched.
    fn index_side_trees(&self, path: &std::path::Path) -> Result<Vec<String>> {
        use gix::objs::tree::{Entry, EntryKind};
        let repo = self.repo();
        let backend = |e: &dyn std::fmt::Display| {
            Error::new(ErrorKind::Backend(format!("index side for LFS fetch: {e}")))
        };
        let relative = path.strip_prefix(&self.spec.workdir).unwrap_or(path);
        let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative));
        let index = repo.index_or_empty().map_err(|e| backend(&e))?;
        let Some(range) = index.entry_range(key.as_ref()) else {
            return Ok(Vec::new());
        };
        let mut trees = Vec::new();
        for entry in &index.entries()[range] {
            let mut components = key.split(|byte| *byte == b'/').rev();
            let Some(name) = components.next() else {
                continue;
            };
            let mut id = repo
                .write_object(gix::objs::Tree {
                    entries: vec![Entry {
                        mode: EntryKind::Blob.into(),
                        filename: name.into(),
                        oid: entry.id,
                    }],
                })
                .map_err(|e| backend(&e))?
                .detach();
            for dir in components {
                id = repo
                    .write_object(gix::objs::Tree {
                        entries: vec![Entry {
                            mode: EntryKind::Tree.into(),
                            filename: dir.into(),
                            oid: id,
                        }],
                    })
                    .map_err(|e| backend(&e))?
                    .detach();
            }
            trees.push(id.to_string());
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
            outputs.push(run_git_with_output(attributes, "git add .gitattributes")?);
            let mut add = self.git_workdir_cmd();
            add.args(["--literal-pathspecs", "add", "--renormalize", "--"])
                .args(renormalize);
            outputs.push(run_git_with_output(add, "git add --renormalize")?);
        }
        Ok(combine("git lfs track", outputs))
    }

    /// Loads on its own when lockable patterns exist, so it runs as a
    /// background read: a server that needs credentials simply fails.
    pub(super) fn lfs_locks_impl(&self, cancellation: &CancellationToken) -> Result<Vec<LfsLock>> {
        let json = run_git_background_capture(
            self.git_lfs(&["locks", "--json"]),
            "git lfs locks --json",
            cancellation,
        )?;
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
