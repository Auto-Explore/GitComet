//! "Apply change": one file's change from a commit or comparison, applied like
//! a cherry-pick narrowed to that file.

use super::GixRepo;
use super::history::{append_command_output, append_raw_output, gix_head_id_or_none};
use super::patch::write_patch_file;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_capture_bytes,
    run_git_raw_output, run_git_with_output, validate_hex_commit_id,
};
use gitcomet_core::domain::{CommitId, DiffTarget};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{
    APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL, APPLY_FILE_CHANGE_ALREADY_STAGED_SENTINEL,
    CommandOutput, Result, apply_file_change_range_message,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A path's entry kind and object in a tree or the index; `None` when absent.
pub(super) type FileVersion = Option<(gix::object::tree::EntryKind, gix::ObjectId)>;

/// What an earlier apply left staged, so a rerun whose commit step failed (an
/// auth retry) commits it instead of applying the change twice.
pub(super) struct StagedAppliedChange {
    target: DiffTarget,
    staged: Vec<FileVersion>,
}

enum CommitMessage<'a> {
    /// Message and authorship of the source commit, as cherry-pick keeps them.
    Reuse(&'a CommitId),
    Text(String),
}

struct ChangeSource<'a> {
    /// `None` for a root commit.
    base: Option<gix::ObjectId>,
    tip: gix::ObjectId,
    path: &'a Path,
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
}

/// Renames the file in a single-file patch's headers, so a change made under
/// an old name applies to the file as it is named now. Only headers before the
/// first hunk are touched.
fn retarget_patch(patch: &[u8], from: &Path, to: &Path) -> Result<Vec<u8>> {
    let from = repo_path_bytes(from);
    let to = repo_path_bytes(to);
    let line = |parts: &[&[u8]]| parts.concat();
    let replacements = [
        (
            line(&[b"diff --git a/", &from, b" b/", &from]),
            line(&[b"diff --git a/", &to, b" b/", &to]),
        ),
        (line(&[b"--- a/", &from]), line(&[b"--- a/", &to])),
        (line(&[b"+++ b/", &from]), line(&[b"+++ b/", &to])),
    ];
    let mut out = Vec::with_capacity(patch.len() + 3 * to.len());
    let mut in_header = true;
    let mut retargeted = false;
    for raw in patch.split_inclusive(|byte| *byte == b'\n') {
        let text = raw.strip_suffix(b"\n").unwrap_or(raw);
        if text.starts_with(b"@@") || text.starts_with(b"GIT binary patch") {
            in_header = false;
        }
        match replacements
            .iter()
            .find(|(old, _)| in_header && text == old.as_slice())
        {
            Some((_, new)) => {
                retargeted |= new.starts_with(b"diff --git");
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
        .and_then(|commit| commit.tree().map_err(Into::into))
        .map_err(|e| backend_error(format!("reading {commit}: {e}")))?;
    let entry = tree
        .lookup_entry_by_path(path)
        .map_err(|e| backend_error(format!("reading {} at {commit}: {e}", path.display())))?;
    Ok(entry.map(|entry| (entry.mode().kind(), entry.object_id())))
}

impl GixRepo {
    pub(super) fn apply_file_change_with_output_impl(
        &self,
        target: &DiffTarget,
        commit: bool,
    ) -> Result<CommandOutput> {
        let source = self.change_source(target)?;
        if let Some(operation) = self.operation_in_progress_label() {
            return Err(backend_error(format!(
                "{operation} is in progress; finish or abort it first"
            )));
        }
        let display_path = source.path.display().to_string();
        let repo = self.repo();
        let names = self.file_names(&repo, target, &source)?;
        let paths: Vec<PathBuf> = names.iter().map(|name| name.in_checkout.clone()).collect();

        let head = gix_head_id_or_none(&repo)?;
        let mut post = Vec::new();
        let mut in_head = Vec::new();
        for name in &names {
            post.push(tree_version(&repo, Some(source.tip), &name.in_change)?);
            in_head.push(tree_version(&repo, head, &name.in_checkout)?);
        }
        let staged = self.index_versions(&paths)?;

        let label = format!("git apply --3way {display_path}");
        let done = |sentinel: &str| CommandOutput {
            command: label.clone(),
            stdout: sentinel.to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        };
        if staged == post && in_head == post {
            return Ok(done(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL));
        }
        // `git apply --index` and `git commit --only` both take the worktree
        // copy, so it must be what the index holds.
        if !self.worktree_matches_index(&paths)? {
            return Err(backend_error(format!(
                "{display_path} has unstaged changes; stage, stash or discard them first"
            )));
        }

        let mut output = done("");
        let staged_by_earlier_apply = staged == post
            || self
                .staged_applied_change
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|earlier| &earlier.target == target && earlier.staged == staged);
        if staged_by_earlier_apply {
            if !commit {
                return Ok(done(APPLY_FILE_CHANGE_ALREADY_STAGED_SENTINEL));
            }
        } else {
            // Committing just this change needs the file's index to be HEAD's.
            if staged != in_head {
                return Err(backend_error(format!(
                    "{display_path} has staged changes; commit or unstage them first"
                )));
            }
            let applied = self.apply_patch_3way(&source, &names, &label)?;
            let staged = self.index_versions(&paths)?;
            if staged == in_head {
                // The 3-way merge found the change already there.
                return Ok(done(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL));
            }
            append_raw_output(&mut output, &applied);
            *self
                .staged_applied_change
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(StagedAppliedChange {
                target: target.clone(),
                staged,
            });
        }

        if commit {
            let commit_output = self.commit_applied_change(&source, &paths)?;
            output.command = format!("{label} && {}", commit_output.command);
            append_command_output(&mut output, commit_output);
            *self
                .staged_applied_change
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(output)
    }

    fn change_source<'a>(&self, target: &'a DiffTarget) -> Result<ChangeSource<'a>> {
        let repo = self.repo();
        let peel = |id: &CommitId| -> Result<gix::Commit<'_>> {
            validate_hex_commit_id(id)?;
            super::history::peel_commit(&repo, id.as_ref())
        };
        match target {
            DiffTarget::Commit { commit_id, path } => {
                let tip = peel(commit_id)?;
                Ok(ChangeSource {
                    // First parent: the diff the panel shows for a commit.
                    base: tip.parent_ids().next().map(|id| id.detach()),
                    tip: tip.id,
                    path,
                    message: CommitMessage::Reuse(commit_id),
                })
            }
            DiffTarget::CommitRange {
                from_commit_id,
                to_commit_id: Some(to_commit_id),
                path: Some(path),
            } => Ok(ChangeSource {
                base: Some(peel(from_commit_id)?.id),
                tip: peel(to_commit_id)?.id,
                path,
                message: CommitMessage::Text(apply_file_change_range_message(
                    from_commit_id,
                    to_commit_id,
                    path,
                )),
            }),
            _ => Err(backend_error(
                "only a file's change in a commit or between two commits can be applied"
                    .to_string(),
            )),
        }
    }

    /// The files the change touches, by name in the change and in the checkout.
    fn file_names(
        &self,
        repo: &gix::Repository,
        target: &DiffTarget,
        source: &ChangeSource<'_>,
    ) -> Result<Vec<FileName>> {
        let touched = |path: &Path| -> Result<bool> {
            Ok(tree_version(repo, source.base, path)?
                != tree_version(repo, Some(source.tip), path)?)
        };
        if touched(source.path)? {
            let mut names = Vec::with_capacity(2);
            if let Some(old) = self.renamed_from(source)? {
                names.push(FileName::same(old));
            }
            names.push(FileName::same(source.path.to_path_buf()));
            return Ok(names);
        }
        // A file-history row from before a rename: the commit changed the file
        // under the name it had then.
        if let DiffTarget::Commit { commit_id, .. } = target
            && let Some(then) = self.resolve_file_path_at_commit_impl(source.path, commit_id)?
            && then != source.path
            && touched(&then)?
        {
            return Ok(vec![FileName {
                in_change: then,
                in_checkout: source.path.to_path_buf(),
            }]);
        }
        Err(backend_error(format!(
            "there are no changes to {} to apply",
            source.path.display()
        )))
    }

    /// The path the change renamed `source.path` from, found the way the
    /// commit diff finds it: rename detection over the whole change.
    fn renamed_from(&self, source: &ChangeSource<'_>) -> Result<Option<PathBuf>> {
        let Some(base) = source.base else {
            return Ok(None);
        };
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["diff-tree", "-r", "-M", "-z", "--name-status"])
            .arg(base.to_string())
            .arg(source.tip.to_string());
        let listing = run_git_capture_bytes(cmd, "git diff-tree -M --name-status")?;
        let wanted = repo_path_bytes(source.path);
        let mut fields = listing.split(|byte| *byte == 0);
        while let Some(status) = fields.next() {
            let Some(first) = fields.next() else {
                break;
            };
            if status.first() == Some(&b'R') || status.first() == Some(&b'C') {
                let Some(second) = fields.next() else {
                    break;
                };
                if status.first() == Some(&b'R') && second == wanted.as_slice() {
                    return Ok(Some(gix::path::from_byte_slice(first).to_path_buf()));
                }
            }
        }
        Ok(None)
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

    fn worktree_matches_index(&self, paths: &[PathBuf]) -> Result<bool> {
        let label = "git diff --quiet";
        let mut cmd = self.literal_paths_cmd();
        cmd.args(["diff", "--no-ext-diff", "--no-textconv", "--quiet", "--"])
            .args(paths);
        let output = run_git_raw_output(cmd, label)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_command_failed_error(label, output)),
        }
    }

    /// The change as a patch `git apply --3way` replays. Plumbing, so the
    /// user's diff config (context size, colour, prefixes) cannot reshape it,
    /// with full blob ids for the 3-way fallback.
    fn change_patch(&self, source: &ChangeSource<'_>, names: &[FileName]) -> Result<Vec<u8>> {
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
        cmd.arg(source.tip.to_string())
            .arg("--")
            .args(names.iter().map(|name| &name.in_change));
        let patch = run_git_capture_bytes(cmd, "git diff-tree -p")?;
        match names {
            [name] if name.in_change != name.in_checkout => {
                retarget_patch(&patch, &name.in_change, &name.in_checkout)
            }
            _ => Ok(patch),
        }
    }

    fn apply_patch_3way(
        &self,
        source: &ChangeSource<'_>,
        names: &[FileName],
        label: &str,
    ) -> Result<std::process::Output> {
        let patch_file = write_patch_file(&self.change_patch(source, names)?)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["apply", "--3way", "--whitespace=nowarn"])
            .arg(patch_file.path());
        let applied = run_git_raw_output(cmd, label)?;
        if applied.status.success() {
            return Ok(applied);
        }
        // Conflicts leave unmerged entries for the resolver.
        let paths: Vec<PathBuf> = names.iter().map(|name| name.in_checkout.clone()).collect();
        if self.index_versions(&paths).is_err() {
            let display_path = source.path.display();
            let detail = format!(
                "Applied the change to {display_path} with conflicts. Resolve them, then commit.\n\n{}",
                bytes_to_text_preserving_utf8(&applied.stderr).trim()
            );
            return Err(Error::new(ErrorKind::Git(GitFailure::new(
                label,
                GitFailureId::ApplyChangeConflict,
                applied.status.code(),
                applied.stdout,
                applied.stderr,
                Some(detail.trim_end().to_string()),
            ))));
        }
        Err(git_command_failed_error(label, applied))
    }

    /// Commits exactly `paths`; `--only` leaves other files' staged work out,
    /// and the caller made sure these paths hold nothing but the change.
    fn commit_applied_change(
        &self,
        source: &ChangeSource<'_>,
        paths: &[PathBuf],
    ) -> Result<CommandOutput> {
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
        cmd.arg("--").args(paths);
        run_git_with_output(cmd, &label).map_err(|error| match error.kind() {
            // The change stays staged; mark it so the store offers the message.
            ErrorKind::Git(failure) if failure.id() == GitFailureId::CommandFailed => {
                Error::new(ErrorKind::Git(GitFailure::new(
                    failure.command(),
                    GitFailureId::ApplyChangeCommitFailed,
                    failure.exit_code(),
                    failure.stdout().to_vec(),
                    failure.stderr().to_vec(),
                    failure.detail().map(str::to_owned),
                )))
            }
            _ => error,
        })
    }
}
