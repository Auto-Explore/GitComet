//! "Apply change": files' change from a commit or comparison, applied like
//! a cherry-pick narrowed to those files.

use super::GixRepo;
use super::history::{
    append_command_output, append_raw_output, gix_head_id_or_none, pathspec_from_file_arg,
};
use super::patch::{write_patch_file, write_pathspec_file};
use crate::util::{
    bytes_to_text_preserving_utf8, describe_path_list, git_command_failed_error,
    run_git_capture_bytes, run_git_raw_output, run_git_with_output, validate_hex_commit_id,
};
use gitcomet_core::domain::{
    ApplyChangeSource, ApplyChangeTarget, ApplyFileChangeIndexEntry, ApplyFileChangeRetry, CommitId,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{
    APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL, CommandOutput, Result,
    apply_file_change_range_message,
};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A path's entry kind and object in a tree or the index; `None` when absent.
type FileVersion = Option<(gix::object::tree::EntryKind, gix::ObjectId)>;

/// Paths per git invocation, so a large selection stays under Windows'
/// command-line limit.
const PATHS_PER_COMMAND: usize = 200;

enum CommitMessage<'a> {
    /// Message and authorship of the source commit, as cherry-pick keeps them.
    Reuse(&'a CommitId),
    Text(String),
}

struct ChangeSource<'a> {
    /// `None` for a root commit.
    base: Option<gix::ObjectId>,
    tip: gix::ObjectId,
    message: CommitMessage<'a>,
}

/// One file the change touches, by its name in the change's trees and in the
/// checkout. They differ for a file-history row from before a rename, which
/// names the file as it is called now.
struct FileName {
    in_change: PathBuf,
    in_checkout: PathBuf,
}

impl FileName {
    fn same(path: PathBuf) -> Self {
        Self {
            in_change: path.clone(),
            in_checkout: path,
        }
    }

    fn retargeted(&self) -> bool {
        self.in_change != self.in_checkout
    }
}

/// Renames the file in a single-file patch's headers, so a change made under
/// an old name applies to the file as it is named now. Only headers before the
/// first hunk are touched.
fn retarget_patch(patch: &[u8], to: &Path) -> Result<Vec<u8>> {
    let to = repo_path_bytes(to);
    let line = |parts: &[&[u8]]| parts.concat();
    let old_path = gix::quote::ansi_c::quote(line(&[b"a/", &to]).as_slice().into()).into_owned();
    let new_path = gix::quote::ansi_c::quote(line(&[b"b/", &to]).as_slice().into()).into_owned();
    // The patch is generated for exactly one file. Rebuild its path headers
    // instead of matching Git's optional quoting and trailing TAB byte for
    // byte. A TAB terminates an unquoted path containing spaces.
    let headers = [
        line(&[b"diff --git ", &old_path, b" ", &new_path]),
        line(&[b"--- ", &old_path, b"\t"]),
        line(&[b"+++ ", &new_path, b"\t"]),
    ];
    let mut out = Vec::with_capacity(patch.len() + 3 * to.len());
    let mut in_header = true;
    let mut retargeted = false;
    for raw in patch.split_inclusive(|byte| *byte == b'\n') {
        let text = raw.strip_suffix(b"\n").unwrap_or(raw);
        if text.starts_with(b"@@") || text.starts_with(b"GIT binary patch") {
            in_header = false;
        }
        let replacement = if !in_header {
            None
        } else if text.starts_with(b"diff --git ") {
            retargeted = true;
            Some(&headers[0])
        } else if text.starts_with(b"--- ") && text != b"--- /dev/null" {
            Some(&headers[1])
        } else if text.starts_with(b"+++ ") && text != b"+++ /dev/null" {
            Some(&headers[2])
        } else {
            None
        };
        match replacement {
            Some(new) => {
                out.extend_from_slice(new);
                out.extend_from_slice(&raw[text.len()..]);
            }
            None => out.extend_from_slice(raw),
        }
    }
    if !retargeted {
        return Err(backend_error(format!(
            "cannot apply a change made to {} under its old name",
            gix::path::from_byte_slice(&to).display()
        )));
    }
    Ok(out)
}

fn backend_error(message: String) -> Error {
    Error::new(ErrorKind::Backend(format!("apply change: {message}")))
}

fn repo_path_bytes(path: &Path) -> Vec<u8> {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path)).to_vec()
}

fn tree_version(
    repo: &gix::Repository,
    commit: Option<gix::ObjectId>,
    path: &Path,
) -> Result<FileVersion> {
    let Some(commit) = commit else {
        return Ok(None);
    };
    let tree = repo
        .find_commit(commit)
        .and_then(|commit| commit.tree())
        .map_err(|e| backend_error(format!("reading {commit}: {e}")))?;
    let entry = tree
        .lookup_entry_by_path(path)
        .map_err(|e| backend_error(format!("reading {} at {commit}: {e}", path.display())))?;
    Ok(entry.map(|entry| (entry.mode().kind(), entry.object_id())))
}

impl GixRepo {
    pub(super) fn apply_file_change_with_output_impl(
        &self,
        target: &ApplyChangeTarget,
        commit: bool,
        retry: Option<&ApplyFileChangeRetry>,
    ) -> Result<CommandOutput> {
        if target.paths.is_empty() {
            return Err(backend_error("no files to apply".to_string()));
        }
        let source = self.change_source(target)?;
        if let Some(operation) = self.operation_in_progress_label()? {
            return Err(backend_error(format!(
                "{operation} is in progress; finish or abort it first"
            )));
        }
        let repo = self.repo();
        // One group per requested file: a renamed file brings its old name.
        let groups = self.file_names(&repo, target, &source)?;
        let names: Vec<&FileName> = groups.iter().flatten().collect();
        let paths: Vec<PathBuf> = names.iter().map(|name| name.in_checkout.clone()).collect();

        let head = gix_head_id_or_none(&repo)?;
        let mut post = Vec::new();
        let mut in_head = Vec::new();
        for name in &names {
            post.push(tree_version(&repo, Some(source.tip), &name.in_change)?);
            in_head.push(tree_version(&repo, head, &name.in_checkout)?);
        }
        let staged = self.index_versions(&paths)?;
        if post
            .iter()
            .chain(&in_head)
            .chain(&staged)
            .any(|version| matches!(version, Some((gix::object::tree::EntryKind::Commit, _))))
        {
            return Err(backend_error(
                "submodule changes cannot be applied as file changes".into(),
            ));
        }

        let label = match target.paths.as_slice() {
            [path] => format!("git apply --3way {}", path.display()),
            paths => format!("git apply --3way {} files", paths.len()),
        };
        let done = |sentinel: &str| CommandOutput {
            command: label.clone(),
            stdout: sentinel.to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        };
        // `git apply --index` and `git commit --only` both take the worktree
        // copy, so it must be what the index holds.
        let unstaged = self.paths_with_unstaged_changes(&paths, &staged)?;
        if !unstaged.is_empty() {
            return Err(backend_error(format!(
                "{} {} unstaged changes; stage, stash or discard them first",
                describe_paths(&unstaged),
                has_or_have(unstaged.len())
            )));
        }

        // A group the branch already has, staged and committed, needs no patch.
        let mut ix = 0;
        let pending: Vec<&[FileName]> = groups
            .iter()
            .filter(|group| {
                let range = ix..ix + group.len();
                ix = range.end;
                !range
                    .into_iter()
                    .all(|i| staged[i] == post[i] && in_head[i] == post[i])
            })
            .map(Vec::as_slice)
            .collect();
        if pending.is_empty() {
            return Ok(done(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL));
        }

        let mut output = done("");
        let checkpoint = if let Some(retry) = retry {
            let current = apply_checkpoint(target, head, &paths, &staged);
            if current.index == retry.index && staged == in_head {
                return Ok(done(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL));
            }
            if &current != retry {
                return Err(backend_error(
                    "HEAD or the staged change has changed since the failed commit; review and commit it manually".into(),
                ));
            }
            current
        } else {
            // Committing just this change needs each file's index to be HEAD's.
            let staged_paths: Vec<&Path> = names
                .iter()
                .enumerate()
                .filter(|(i, _)| staged[*i] != in_head[*i])
                .map(|(_, name)| name.in_checkout.as_path())
                .collect();
            if !staged_paths.is_empty() {
                return Err(backend_error(format!(
                    "{} {} staged changes; commit or unstage them first",
                    describe_paths(&staged_paths),
                    has_or_have(staged_paths.len())
                )));
            }
            let applied = self.apply_patch_3way(&source, &pending, &paths, &label)?;
            let staged = self.index_versions(&paths)?;
            if staged == in_head {
                // The 3-way merge found the change already there.
                return Ok(done(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL));
            }
            append_raw_output(&mut output, &applied);
            apply_checkpoint(target, head, &paths, &staged)
        };

        if commit {
            let commit_output = self.commit_applied_change(&source, &paths, checkpoint)?;
            output.command = format!("{label} && {}", commit_output.command);
            append_command_output(&mut output, commit_output);
        }
        Ok(output)
    }

    fn change_source<'a>(&self, target: &'a ApplyChangeTarget) -> Result<ChangeSource<'a>> {
        let repo = self.repo();
        let peel = |id: &CommitId| -> Result<gix::Commit<'_>> {
            validate_hex_commit_id(id)?;
            super::history::peel_commit(&repo, id.as_ref())
        };
        match &target.source {
            ApplyChangeSource::Commit(commit_id) => {
                let tip = peel(commit_id)?;
                Ok(ChangeSource {
                    // First parent: the diff the panel shows for a commit.
                    base: tip.parent_ids().next().map(|id| id.detach()),
                    tip: tip.id,
                    message: CommitMessage::Reuse(commit_id),
                })
            }
            ApplyChangeSource::Range { from, to } => Ok(ChangeSource {
                base: Some(peel(from)?.id),
                tip: peel(to)?.id,
                message: CommitMessage::Text(apply_file_change_range_message(
                    from,
                    to,
                    &target.paths,
                )),
            }),
        }
    }

    /// The files the change touches, one group per requested path, by name
    /// in the change and in the checkout. A path requested twice, directly
    /// or as a rename's source, is applied once.
    fn file_names(
        &self,
        repo: &gix::Repository,
        target: &ApplyChangeTarget,
        source: &ChangeSource<'_>,
    ) -> Result<Vec<Vec<FileName>>> {
        let touched = |path: &Path| -> Result<bool> {
            Ok(tree_version(repo, source.base, path)?
                != tree_version(repo, Some(source.tip), path)?)
        };
        let mut renames: Option<FxHashMap<PathBuf, PathBuf>> = None;
        let mut groups: Vec<Vec<FileName>> = Vec::with_capacity(target.paths.len());
        let mut seen = rustc_hash::FxHashSet::default();
        for path in &target.paths {
            let group = if touched(path)? {
                let mut names = Vec::with_capacity(2);
                if tree_version(repo, source.base, path)?.is_none() {
                    if renames.is_none() {
                        renames = Some(self.renames(source)?);
                    }
                    if let Some(old) = renames.as_ref().and_then(|map| map.get(path)) {
                        names.push(FileName::same(old.clone()));
                    }
                }
                names.push(FileName::same(path.clone()));
                names
            } else {
                // A file-history row from before a rename: the commit changed
                // the file under the name it had then.
                match &target.source {
                    ApplyChangeSource::Commit(commit_id) => {
                        match self.resolve_file_path_at_commit_impl(path, commit_id)? {
                            Some(then) if &then != path && touched(&then)? => {
                                vec![FileName {
                                    in_change: then,
                                    in_checkout: path.clone(),
                                }]
                            }
                            _ => return Err(no_changes_error(path)),
                        }
                    }
                    ApplyChangeSource::Range { .. } => return Err(no_changes_error(path)),
                }
            };
            if group
                .iter()
                .all(|name| seen.insert(name.in_checkout.clone()))
            {
                groups.push(group);
            }
        }
        Ok(groups)
    }

    /// New path → old path for every rename in the change, found the way the
    /// commit diff finds them: rename detection over the whole change.
    fn renames(&self, source: &ChangeSource<'_>) -> Result<FxHashMap<PathBuf, PathBuf>> {
        let mut renames = FxHashMap::default();
        let Some(base) = source.base else {
            return Ok(renames);
        };
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["diff-tree", "-r", "-M", "-z", "--name-status"])
            .arg(base.to_string())
            .arg(source.tip.to_string());
        let listing = run_git_capture_bytes(cmd, "git diff-tree -M --name-status")?;
        let mut fields = listing.split(|byte| *byte == 0);
        while let Some(status) = fields.next() {
            let Some(first) = fields.next() else {
                break;
            };
            if status.first() == Some(&b'R') || status.first() == Some(&b'C') {
                let Some(second) = fields.next() else {
                    break;
                };
                if status.first() == Some(&b'R') {
                    renames.insert(
                        gix::path::from_byte_slice(second).to_path_buf(),
                        gix::path::from_byte_slice(first).to_path_buf(),
                    );
                }
            }
        }
        Ok(renames)
    }

    /// Stage-0 versions of `paths` in a fresh read of the index. A conflicted
    /// path is refused: there is no single version to apply on top of.
    fn index_versions(&self, paths: &[PathBuf]) -> Result<Vec<FileVersion>> {
        let index = self
            .repo()
            .open_index()
            .map_err(|e| backend_error(format!("reading the index: {e}")))?;
        paths
            .iter()
            .map(|path| {
                let key = repo_path_bytes(path);
                match index.entry_by_path(key.as_slice().into()) {
                    None => Ok(None),
                    Some(entry) if entry.stage_raw() != 0 => Err(backend_error(format!(
                        "{} has unresolved conflicts; resolve them first",
                        path.display()
                    ))),
                    Some(entry) => Ok(entry
                        .mode
                        .to_tree_entry_mode()
                        .map(|mode| (mode.kind(), entry.id))),
                }
            })
            .collect()
    }

    fn literal_paths_cmd(&self) -> Command {
        let mut cmd = self.git_workdir_cmd();
        cmd.env("GIT_LITERAL_PATHSPECS", "1");
        cmd
    }

    /// The `paths` whose worktree copy differs from the index.
    fn paths_with_unstaged_changes<'p>(
        &self,
        paths: &'p [PathBuf],
        staged: &[FileVersion],
    ) -> Result<Vec<&'p Path>> {
        let mut dirty = rustc_hash::FxHashSet::default();
        // `git diff` omits untracked (including ignored) files. `commit --only`
        // would nevertheless add one left on disk after `git rm --cached`.
        for (path, version) in paths.iter().zip(staged) {
            if version.is_none() {
                match std::fs::symlink_metadata(self.spec.workdir.join(path)) {
                    Ok(_) => {
                        dirty.insert(path.as_path());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::new(ErrorKind::Io(error.kind()))),
                }
            }
        }
        for chunk in paths.chunks(PATHS_PER_COMMAND) {
            let mut cmd = self.literal_paths_cmd();
            cmd.args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--name-only",
                "-z",
                "--",
            ])
            .args(chunk);
            let listing = run_git_capture_bytes(cmd, "git diff --name-only")?;
            for name in listing
                .split(|byte| *byte == 0)
                .filter(|name| !name.is_empty())
            {
                let name = gix::path::from_byte_slice(name);
                if let Some(path) = chunk.iter().find(|path| path.as_path() == name) {
                    dirty.insert(path.as_path());
                }
            }
        }
        Ok(paths
            .iter()
            .map(PathBuf::as_path)
            .filter(|path| dirty.contains(path))
            .collect())
    }

    /// The change as a patch `git apply --3way` replays. Plumbing, so the
    /// user's diff config (context size, colour, prefixes) cannot reshape it,
    /// with full blob ids for the 3-way fallback. A file named differently in
    /// the checkout gets a patch of its own with retargeted headers.
    fn change_patch(&self, source: &ChangeSource<'_>, groups: &[&[FileName]]) -> Result<Vec<u8>> {
        let (retargeted, plain): (Vec<&[FileName]>, Vec<&[FileName]>) = groups
            .iter()
            .partition(|group| group.iter().any(FileName::retargeted));
        let mut patch = Vec::new();
        // Groups stay whole within a chunk, so rename detection sees both names.
        let mut chunk: Vec<&Path> = Vec::new();
        for (ix, group) in plain.iter().enumerate() {
            chunk.extend(group.iter().map(|name| name.in_change.as_path()));
            if chunk.len() >= PATHS_PER_COMMAND || ix + 1 == plain.len() {
                patch.extend(self.diff_tree_patch(source, &chunk)?);
                chunk.clear();
            }
        }
        for group in retargeted {
            for name in group {
                let section = self.diff_tree_patch(source, &[name.in_change.as_path()])?;
                patch.extend(retarget_patch(&section, &name.in_checkout)?);
            }
        }
        Ok(patch)
    }

    fn diff_tree_patch(&self, source: &ChangeSource<'_>, names: &[&Path]) -> Result<Vec<u8>> {
        let mut cmd = self.literal_paths_cmd();
        cmd.args([
            "diff-tree",
            "-p",
            "--binary",
            "--full-index",
            "--no-color",
            "-U3",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-M",
        ]);
        match source.base {
            Some(base) => cmd.arg(base.to_string()),
            None => cmd.arg("--root"),
        };
        cmd.arg(source.tip.to_string()).arg("--").args(names);
        run_git_capture_bytes(cmd, "git diff-tree -p")
    }

    fn apply_patch_3way(
        &self,
        source: &ChangeSource<'_>,
        groups: &[&[FileName]],
        paths: &[PathBuf],
        label: &str,
    ) -> Result<std::process::Output> {
        let patch_file = write_patch_file(&self.change_patch(source, groups)?)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["apply", "--3way", "--whitespace=nowarn"])
            .arg(patch_file.path());
        let applied = run_git_raw_output(cmd, label)?;
        if applied.status.success() {
            return Ok(applied);
        }
        // Conflicts leave unmerged entries for the resolver.
        Err(self.failed_apply_error(paths, label, applied))
    }

    fn failed_apply_error(
        &self,
        paths: &[PathBuf],
        label: &str,
        applied: std::process::Output,
    ) -> Error {
        let conflicted: Vec<&Path> = match self.conflicted_paths(paths) {
            Ok(conflicted) => conflicted,
            Err(_) => return git_command_failed_error(label, applied),
        };
        if conflicted.is_empty() {
            return git_command_failed_error(label, applied);
        }
        let changes = if paths.len() == 1 {
            format!("the change to {}", describe_paths(&conflicted))
        } else {
            format!(
                "the changes with conflicts in {}",
                describe_paths(&conflicted)
            )
        };
        let applied_what = if paths.len() == 1 {
            format!("Applied {changes} with conflicts.")
        } else {
            format!("Applied {changes}.")
        };
        let detail = format!(
            "{applied_what} Resolve them, then commit.\n\n{}",
            bytes_to_text_preserving_utf8(&applied.stderr).trim()
        );
        Error::new(ErrorKind::Git(GitFailure::new(
            label,
            GitFailureId::ApplyChangeConflict,
            applied.status.code(),
            applied.stdout,
            applied.stderr,
            Some(detail.trim_end().to_string()),
        )))
    }

    fn conflicted_paths<'p>(&self, paths: &'p [PathBuf]) -> Result<Vec<&'p Path>> {
        let repo = self.repo();
        let index = repo
            .open_index()
            .map_err(|e| backend_error(format!("reading the index: {e}")))?;
        Ok(paths
            .iter()
            .filter(|path| {
                let key = repo_path_bytes(path);
                index
                    .entry_by_path(key.as_slice().into())
                    .is_some_and(|entry| entry.stage_raw() != 0)
            })
            .map(PathBuf::as_path)
            .collect())
    }

    /// Commits exactly `paths`; `--only` leaves other files' staged work out,
    /// and the caller made sure these paths hold nothing but the change.
    fn commit_applied_change(
        &self,
        source: &ChangeSource<'_>,
        paths: &[PathBuf],
        checkpoint: ApplyFileChangeRetry,
    ) -> Result<CommandOutput> {
        let path_bytes: Vec<Vec<u8>> = paths.iter().map(|path| repo_path_bytes(path)).collect();
        let pathspec = write_pathspec_file(path_bytes.iter().map(Vec::as_slice))?;
        let mut cmd = self.literal_paths_cmd();
        cmd.args(["commit", "--no-verify", "--only"]);
        let label = match &source.message {
            CommitMessage::Reuse(commit_id) => {
                cmd.arg("-C").arg(commit_id.as_ref());
                format!("git commit --only -C {}", commit_id.as_ref())
            }
            CommitMessage::Text(text) => {
                cmd.arg("-m").arg(text);
                "git commit --only".to_string()
            }
        };
        cmd.arg("--pathspec-file-nul")
            .arg(pathspec_from_file_arg(pathspec.path()));
        run_git_with_output(cmd, &label).map_err(|error| match error.kind() {
            // The change stays staged; mark it so the store offers the message.
            ErrorKind::Git(failure) if failure.id() == GitFailureId::CommandFailed => {
                Error::new(ErrorKind::Git(
                    GitFailure::new(
                        failure.command(),
                        GitFailureId::ApplyChangeCommitFailed,
                        failure.exit_code(),
                        failure.stdout().to_vec(),
                        failure.stderr().to_vec(),
                        failure.detail().map(str::to_owned),
                    )
                    .with_apply_file_change_retry(checkpoint),
                ))
            }
            _ => error,
        })
    }
}

fn no_changes_error(path: &Path) -> Error {
    backend_error(format!(
        "there are no changes to {} to apply",
        path.display()
    ))
}

fn describe_paths<P: AsRef<Path>>(paths: &[P]) -> String {
    let bytes: Vec<Vec<u8>> = paths
        .iter()
        .map(|path| gix::path::into_bstr(path.as_ref()).to_vec())
        .collect();
    describe_path_list(&bytes)
}

fn has_or_have(count: usize) -> &'static str {
    if count == 1 { "has" } else { "have" }
}

fn apply_checkpoint(
    target: &ApplyChangeTarget,
    head: Option<gix::ObjectId>,
    paths: &[PathBuf],
    staged: &[FileVersion],
) -> ApplyFileChangeRetry {
    ApplyFileChangeRetry {
        target: target.clone(),
        head: head.map(|id| CommitId(id.to_string().into())),
        index: paths
            .iter()
            .zip(staged)
            .map(|(path, version)| ApplyFileChangeIndexEntry {
                path: path.clone(),
                version: version.map(|(mode, id)| (mode as u16, CommitId(id.to_string().into()))),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_apply_with_an_unreadable_index_keeps_the_original_git_error() {
        let dir = tempfile::tempdir().unwrap();
        let git = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), git.into_sync());
        std::fs::write(dir.path().join(".git/index"), b"invalid index").unwrap();
        let paths = vec![PathBuf::from("a.txt")];
        assert!(repo.conflicted_paths(&paths).is_err());
        let mut command = repo.git_workdir_cmd();
        command
            .args(["apply", "--3way", "--"])
            .arg(dir.path().join("missing.patch"));
        let output = run_git_raw_output(command, "git apply --3way").unwrap();
        assert!(!output.status.success());
        let stderr = output.stderr.clone();
        let error = repo.failed_apply_error(&paths, "git apply --3way", output);
        let ErrorKind::Git(failure) = error.kind() else {
            panic!("{error}")
        };
        assert_eq!(failure.id(), GitFailureId::CommandFailed);
        assert_eq!(failure.stderr(), stderr);
        assert!(!error.to_string().contains("with conflicts"));
    }
}
