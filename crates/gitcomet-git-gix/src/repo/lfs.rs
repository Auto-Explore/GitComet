//! Git LFS operations, run through `git lfs` so hooks, credentials and the
//! user's transfer configuration all apply.

use crate::util::{run_git_capture_cancellable, run_git_with_output, validate_ref_like_arg};
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
                lockable,
                renormalize,
            } => self.lfs_track(patterns, *lockable, renormalize),
        }
    }

    /// `git lfs pull` has no pathspec, so fetch exactly these objects, then
    /// check out just these paths.
    fn lfs_pull(&self, paths: &[PathBuf]) -> Result<CommandOutput> {
        if paths.is_empty() {
            return run_git_with_output(self.git_lfs(&["pull"]), "git lfs pull");
        }
        let include = paths
            .iter()
            .map(|path| {
                lfs_include_pattern(path).ok_or_else(|| {
                    Error::new(ErrorKind::Backend(format!(
                        "cannot download `{}` by name: Git LFS patterns cannot contain commas",
                        path.display()
                    )))
                })
            })
            .collect::<Result<Vec<_>>>()?
            .join(",");
        let mut fetch = self.git_lfs(&["fetch"]);
        fetch.arg(format!("--include={include}"));
        let fetched = run_git_with_output(fetch, "git lfs fetch")?;
        let mut checkout = self.git_lfs(&["checkout", "--"]);
        checkout.args(paths);
        let checked_out = run_git_with_output(checkout, "git lfs checkout")?;
        Ok(combine("git lfs pull", vec![fetched, checked_out]))
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
        lockable: bool,
        renormalize: &[PathBuf],
    ) -> Result<CommandOutput> {
        if patterns.is_empty() {
            return Err(paths_arg_error("git lfs track"));
        }
        let mut track = self.git_lfs(&["track"]);
        if lockable {
            track.arg("--lockable");
        }
        track.arg("--").args(patterns);
        let mut outputs = vec![run_git_with_output(track, "git lfs track")?];
        if !renormalize.is_empty() {
            // The new attributes must be staged with the files they convert.
            let mut add = self.git_workdir_cmd();
            add.args(["add", "--renormalize", "--", ".gitattributes"])
                .args(renormalize);
            outputs.push(run_git_with_output(add, "git add --renormalize")?);
        }
        Ok(combine("git lfs track", outputs))
    }

    pub(super) fn lfs_locks_impl(&self, cancellation: &CancellationToken) -> Result<Vec<LfsLock>> {
        let json = run_git_capture_cancellable(
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
