//! The `gitcomet/workspace` branch and the virtual-branch operations behind it.
//!
//! The workspace branch holds the combined tree of every applied virtual
//! branch, so a user can have several pieces of work in one working directory
//! without checking any of them out. It is rebuilt from scratch rather than
//! patched: applying, unapplying, or restacking any branch replays the same
//! sequence of merges from the target, which means the branch can always be
//! reconstructed from the virtual-branch set alone and never accumulates merge
//! cruft.
//!
//! Every command here is a plain `git` invocation through the shared helpers,
//! matching the rest of this backend. The plumbing is used deliberately:
//! `merge-tree` writes no ref and does not touch the index or worktree, so a
//! rebuild can be computed and then discarded without the user ever seeing an
//! intermediate state.

use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind, Result};
use gitcomet_core::services::{CommandOutput, CommitOperationOutcome};
use gitcomet_core::workspace::{
    hunk_spans, synthesize_for_branch, AssignmentIndex, VirtualBranch, WORKSPACE_BRANCH,
    WORKSPACE_REF_PREFIX, WorkspaceState,
};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::GixRepo;
use crate::util::{run_git_capture, run_git_capture_bytes, run_git_simple};

/// Directory GitComet's own repository files live in, under the shared
/// `.git` directory so linked worktrees see the same workspace.
const WORKSPACE_DIR: &str = "gitcomet";

const WORKSPACE_STATE_FILE: &str = "workspace.json";
const WORKSPACE_ASSIGNMENTS_FILE: &str = "assignments.json";

impl GixRepo {
    /// The `.git` directory of the main worktree, where GitComet's own files
    /// live.
    fn workspace_dir(&self) -> PathBuf {
        self.common_dir_impl().join(WORKSPACE_DIR)
    }

    fn workspace_state_path(&self) -> PathBuf {
        self.workspace_dir().join(WORKSPACE_STATE_FILE)
    }

    fn workspace_assignments_path(&self) -> PathBuf {
        self.workspace_dir().join(WORKSPACE_ASSIGNMENTS_FILE)
    }

    /// A `git` command that reads the target and target-ish revision without
    /// checking anything out.
    fn git_plumbing(&self) -> Command {
        self.git_workdir_cmd()
    }

    /// Resolve a revision to a commit id, or `None` if it does not exist.
    fn resolve_revision(&self, revision: &str) -> Result<Option<CommitId>> {
        let mut cmd = self.git_plumbing();
        cmd.arg("rev-parse").arg("--verify").arg("--quiet");
        cmd.arg(format!("{revision}^{{commit}}"));
        match run_git_capture(cmd, "git rev-parse") {
            Ok(out) => {
                let id = out.trim();
                Ok((!id.is_empty()).then(|| CommitId(id.into())))
            }
            // `--quiet` makes a missing revision exit non-zero with no stderr,
            // which is the "does not exist" case rather than a failure.
            Err(_) => Ok(None),
        }
    }

    fn require_revision(&self, revision: &str) -> Result<CommitId> {
        self.resolve_revision(revision)?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "'{revision}' does not name a commit in this repository"
            )))
        })
    }

    // ── State persistence ────────────────────────────────────────

    pub(super) fn read_workspace_impl(&self) -> Result<WorkspaceState> {
        let path = self.workspace_state_path();
        let Ok(bytes) = std::fs::read(&path) else {
            // No workspace yet is the normal state for a repository that has
            // never opened the Workspace view, not an error.
            return Ok(WorkspaceState::default());
        };
        // A workspace file written by a newer or hand-edited version must not
        // make the whole repository unusable; an unparseable file reads as an
        // empty workspace the user rebuilds from the UI.
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    pub(super) fn write_workspace_impl(&self, state: &WorkspaceState) -> Result<()> {
        state.validate()?;
        let dir = self.workspace_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "creating the workspace directory {}: {e}",
                dir.display()
            )))
        })?;
        let json = serde_json::to_vec_pretty(state)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("serializing workspace: {e}"))))?;
        write_atomically(&self.workspace_state_path(), &json)
    }

    pub(super) fn read_assignments_impl(&self) -> Result<AssignmentIndex> {
        let Ok(bytes) = std::fs::read(self.workspace_assignments_path()) else {
            return Ok(AssignmentIndex::default());
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    pub(super) fn write_assignments_impl(&self, index: &AssignmentIndex) -> Result<()> {
        let dir = self.workspace_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "creating the workspace directory {}: {e}",
                dir.display()
            )))
        })?;
        let json = serde_json::to_vec_pretty(index)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("serializing assignments: {e}"))))?;
        write_atomically(&self.workspace_assignments_path(), &json)
    }

    // ── Virtual branches ─────────────────────────────────────────

    pub(super) fn create_virtual_branch_impl(&self, name: &str, base: &str) -> Result<()> {
        let base_id = self.require_revision(base)?;
        let mut cmd = self.git_plumbing();
        cmd.arg("branch")
            .arg("--no-track")
            .arg(name)
            .arg(base_id.as_ref());
        run_git_simple(cmd, "git branch")
    }

    /// Rebase `name` onto `onto` without touching the worktree.
    ///
    /// `--update-refs` moves the branches stacked on `name` with it, which is
    /// what keeps a stack intact when its bottom branch is rebased onto a new
    /// target.
    pub(super) fn rebase_virtual_branch_impl(&self, name: &str, onto: &str) -> Result<()> {
        let onto_id = self.require_revision(onto)?;
        let name_ref = format!("refs/heads/{name}");

        let mut cmd = self.git_plumbing();
        cmd.arg("rebase")
            .arg("--update-refs")
            .arg("--onto")
            .arg(onto_id.as_ref())
            .arg(onto_id.as_ref())
            .arg(&name_ref);
        // `rebase` touches the worktree via its index, and any unrelated dirty
        // file would abort it. Refuse rather than silently including it.
        self.require_clean_index()?;
        run_git_simple(cmd, "git rebase --onto")
    }

    /// Rebase `name` onto `onto` as a history rewrite with no worktree effect.
    ///
    /// Uses `merge-tree` + `commit-tree` rather than `rebase` so a workspace
    /// branch can be restacked while the user has unrelated edits in the
    /// working tree — which is the normal state in a workspace.
    fn restack_branch_onto(&self, name: &str, onto: &str) -> Result<CommitId> {
        let branch_id = self.require_revision(name)?;
        let onto_id = self.require_revision(onto)?;

        // Nothing to replay when the branch is already on the base.
        if branch_id == onto_id {
            return Ok(branch_id);
        }

        let tree = self.merge_tree(&branch_id, &onto_id)?;
        let commit = self.commit_tree(
            &tree,
            &[&onto_id],
            &format!("Rebase {name} onto {onto}"),
        )?;

        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref")
            .arg(&format!("refs/heads/{name}"))
            .arg(commit.as_ref())
            .arg(branch_id.as_ref());
        run_git_simple(cmd, "git update-ref")?;
        Ok(commit)
    }

    // ── The workspace branch ─────────────────────────────────────

    pub(super) fn update_workspace_branch_impl(&self, applied: &[&str]) -> Result<CommitId> {
        let state = self.read_workspace_impl()?;
        let target_id = self.require_revision(&state.target)?;

        // Start from the target every time: the workspace branch is a pure
        // function of the applied set, so a stale merge from a previous applied
        // set cannot survive into the new one.
        let mut head = target_id.clone();
        let mut merged_any = false;

        for name in applied {
            let branch_id = self.require_revision(name)?;
            if branch_id == head {
                // Already contained in what we have merged; adding it again
                // would produce an empty merge commit.
                continue;
            }
            let tree = match self.merge_tree(&branch_id, &head) {
                Ok(tree) => tree,
                Err(error) => {
                    return Err(Error::new(ErrorKind::Backend(format!(
                        "cannot apply '{name}': {error}\n\n\
                         Two applied branches changed the same files. \
                         Unapply one of them, or resolve the conflict before applying it again."
                    ))))
                }
            };
            // Every merge gets its own commit so the workspace history shows
            // which branch contributed what, and so unapplying one later is a
            // replay rather than an edit of history that has been pushed.
            let message = format!("Apply {name}");
            head = self.commit_tree(&tree, &[&head], &message)?;
            merged_any = true;
        }

        if !merged_any {
            // Nothing changed hands, so the workspace branch should point at
            // the target directly rather than accumulate no-op merge commits.
            let existing = self.resolve_revision(WORKSPACE_BRANCH)?;
            if existing.as_ref() == Some(&head) {
                return Ok(head);
            }
        }

        self.update_workspace_ref(&head)?;
        Ok(head)
    }

    /// Write the workspace branch, refusing to move it backwards.
    ///
    /// The old value is passed as `update-ref`'s expected-old argument so a
    /// concurrent rebuild cannot be silently overwritten.
    fn update_workspace_ref(&self, commit: &CommitId) -> Result<()> {
        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref");
        cmd.arg(format!("refs/heads/{WORKSPACE_BRANCH}"));
        cmd.arg(commit.as_ref());
        // The all-zero id means "create only if absent"; a real old value makes
        // the update conditional on the branch not having moved underneath us.
        cmd.arg(match self.resolve_revision(WORKSPACE_BRANCH)? {
            Some(existing) => existing.to_string(),
            None => "0".repeat(40),
        });
        run_git_simple(cmd, "git update-ref")
    }

    pub(super) fn update_workspace_target_impl(&self, target: &str) -> Result<()> {
        let mut state = self.read_workspace_impl()?;
        state.set_target(target)?;
        self.write_workspace_impl(&state)?;

        let target_id = self.require_revision(&state.target)?;
        // Only independent branches move with the target. A stacked branch is
        // rebased by `rebase_virtual_branch_impl` when its base moves, so
        // rebasing it here too would replay it twice.
        for branch in state.branches.clone() {
            if branch.is_independent() {
                self.restack_branch_onto(&branch.name, &state.target)?;
            }
        }

        let applied = state.application_order()?;
        let names: Vec<&str> = applied.iter().map(|b| b.name.as_str()).collect();
        self.update_workspace_branch_impl(&names)
            .map(|_| ())
    }

    // ── Assigning and committing ─────────────────────────────────

    pub(super) fn commit_paths_to_virtual_branch_impl(
        &self,
        name: &str,
        message: &str,
        paths: &[&Path],
    ) -> Result<CommitOperationOutcome> {
        let state = self.read_workspace_impl()?;
        let branch = state.get(name).ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!("'{name}' is not a workspace branch")))
        })?;
        let base = branch.base_branch(&state.target).to_string();
        let base_id = self.require_revision(&base)?;
        let pre_head = self.resolve_revision(name)?;

        // The commit is built against the branch's own base rather than the
        // workspace, so it contains only the assigned files. A temporary index
        // keeps the user's real index and working tree untouched.
        let index_path = self.workspace_dir().join("commit-index");
        std::fs::create_dir_all(self.workspace_dir()).ok();

        let assignments = self.read_assignments_impl()?;
        let tree = self.commit_paths_tree(&index_path, &base_id, paths, name, &assignments)?;
        let commit = self.commit_tree(
            &tree,
            &[&base_id],
            message,
        )?;

        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref")
            .arg(format!("refs/heads/{name}"))
            .arg(commit.as_ref());
        if let Some(pre) = &pre_head {
            cmd.arg(pre.as_ref());
        }
        run_git_simple(cmd, "git update-ref")?;

        let _ = std::fs::remove_file(&index_path);

        // The branch moved, so the workspace branch has to be rebuilt to pick
        // the new commit up.
        let applied = state.application_order()?;
        let names: Vec<&str> = applied.iter().map(|b| b.name.as_str()).collect();
        self.update_workspace_branch_impl(&names)?;

        Ok(CommitOperationOutcome {
            local_branch: Some(name.to_string()),
            pre_head,
            post_head: Some(commit),
        })
    }

    /// A tree with the branch's base plus the given paths' working-tree
    /// contents, computed without touching the real index.
    ///
    /// Paths are taken from the working tree rather than the index so an edit
    /// that has not been staged can still be committed to a branch.
    fn commit_paths_tree(
        &self,
        index_path: &Path,
        base_id: &CommitId,
        paths: &[&Path],
        branch: &str,
        assignments: &AssignmentIndex,
    ) -> Result<CommitId> {
        let env_index = index_path.as_os_str().to_string_lossy().into_owned();

        let mut read_tree = self.git_plumbing();
        read_tree.arg("read-tree").arg(base_id.as_ref());
        read_tree.env("GIT_INDEX_FILE", &env_index);
        run_git_simple(read_tree, "git read-tree")?;

        // Paths whose hunks are all on this branch are taken from the working
        // tree whole, the way they always were. Only a file that has been split
        // across branches needs a synthesized version.
        let mut split: Vec<&Path> = Vec::new();
        let mut plain: Vec<&Path> = Vec::new();
        for path in paths {
            match assignments.file(path) {
                Some(file) if !file.is_whole() => split.push(path),
                _ => plain.push(path),
            }
        }

        // `--add` because a path that is new in the working tree is not yet in
        // the temporary index. Paths git already knows need no `--add`, but
        // passing it is harmless and keeps this to one command.
        if !plain.is_empty() {
            let mut add = self.git_plumbing();
            add.arg("add").arg("--add").arg("--");
            for path in &plain {
                add.arg(path);
            }
            add.env("GIT_INDEX_FILE", &env_index);
            run_git_simple(add, "git add")?;
        }

        for path in split {
            self.stage_split_file(index_path, &env_index, base_id, path, branch, assignments)?;
        }

        let mut write_tree = self.git_plumbing();
        write_tree.arg("write-tree");
        write_tree.env("GIT_INDEX_FILE", &env_index);
        let tree = run_git_capture(write_tree, "git write-tree")?;
        Ok(CommitId(tree.trim().into()))
    }

    /// Stage the version of a split file that belongs to `branch`.
    ///
    /// The content is built in process — base lines everywhere except the hunks
    /// assigned to this branch — and written straight into the temporary index,
    /// so the user's working tree and real index are never involved. A file that
    /// is not valid UTF-8 cannot be split by hunk at all, so it falls back to
    /// being staged whole rather than being mangled by a lossy decode.
    fn stage_split_file(
        &self,
        index_path: &Path,
        env_index: &str,
        base_id: &CommitId,
        path: &Path,
        branch: &str,
        assignments: &AssignmentIndex,
    ) -> Result<()> {
        let base_bytes = self.blob_at(base_id, path)?;
        let working_bytes = std::fs::read(self.spec().workdir.join(path))?;
        let (Ok(base_text), Ok(working_text)) = (
            std::str::from_utf8(&base_bytes),
            std::str::from_utf8(&working_bytes),
        ) else {
            return self.stage_whole_file(env_index, path);
        };

        let spans = hunk_spans(base_text, working_text);
        let mine: Vec<_> = assignments
            .file(path)
            .map(|file| file.hunks().collect())
            .unwrap_or_default();
        // The fingerprints are recomputed here, against *this branch's* base.
        // That is the whole point of keying on content: the assignment was made
        // from a diff against something else, and it still resolves.
        let keep = |fingerprint: gitcomet_core::workspace::HunkFingerprint| {
            mine.iter()
                .any(|(assigned, assigned_branch)| {
                    *assigned == fingerprint && assigned_branch == branch
                })
        };
        let synthesized = synthesize_for_branch(base_text, working_text, &spans, &keep);

        let scratch = index_path.with_extension("blob");
        std::fs::write(&scratch, synthesized.as_bytes())?;
        let blob = self.hash_object(&scratch);
        let _ = std::fs::remove_file(&scratch);
        let blob = blob?;

        let mode = self.path_mode(base_id, path)?;
        let mut update = self.git_plumbing();
        update
            .arg("update-index")
            .arg("--add")
            .arg("--cacheinfo")
            .arg(format!("{mode},{blob},{}", path.to_string_lossy()))
            .env("GIT_INDEX_FILE", env_index);
        run_git_simple(update, "git update-index")
    }

    /// Stage a file from the working tree, ignoring the split.
    fn stage_whole_file(&self, env_index: &str, path: &Path) -> Result<()> {
        let mut add = self.git_plumbing();
        add.arg("add").arg("--add").arg("--").arg(path);
        add.env("GIT_INDEX_FILE", env_index);
        run_git_simple(add, "git add")
    }

    /// A path's blob as of `revision`, or empty for a file the revision does not
    /// have — a new file splits against an empty base.
    fn blob_at(&self, revision: &CommitId, path: &Path) -> Result<Vec<u8>> {
        let mut cmd = self.git_plumbing();
        cmd.arg("show")
            .arg(format!("{}:{}", revision.as_ref(), path.to_string_lossy()));
        Ok(run_git_capture_bytes(cmd, "git show").unwrap_or_default())
    }

    /// Write `scratch` into the object database and return its id.
    fn hash_object(&self, scratch: &Path) -> Result<CommitId> {
        let mut cmd = self.git_plumbing();
        cmd.arg("hash-object").arg("-w").arg("--").arg(scratch);
        let out = run_git_capture(cmd, "git hash-object")?;
        Ok(CommitId(out.trim().into()))
    }

    /// The mode a path should be staged with.
    ///
    /// The user's real index wins, because that is where a mode they staged
    /// lives; then the base tree; then a plain file. Splitting a file must not
    /// quietly turn an executable into a regular one.
    fn path_mode(&self, base_id: &CommitId, path: &Path) -> Result<&'static str> {
        let mut staged = self.git_plumbing();
        staged
            .arg("ls-files")
            .arg("-s")
            .arg("--")
            .arg(path);
        if let Ok(out) = run_git_capture(staged, "git ls-files")
            && let Some(mode) = parse_mode(&out)
        {
            return Ok(mode);
        }
        let mut base = self.git_plumbing();
        base.arg("ls-tree")
            .arg(base_id.as_ref())
            .arg("--")
            .arg(path);
        if let Ok(out) = run_git_capture(base, "git ls-tree")
            && let Some(mode) = parse_mode(&out)
        {
            return Ok(mode);
        }
        Ok("100644")
    }

    // ── Plumbing helpers ─────────────────────────────────────────

    /// Three-way merge of `theirs` into `ours`, returning the resulting tree.
    ///
    /// `git merge-tree --write-tree` computes the merge entirely in the object
    /// database: no ref moves, no index entry is written, and the worktree is
    /// not read or written. That is what lets the workspace be rebuilt while
    /// the user has unsaved edits in the files being merged.
    ///
    /// Requires Git 2.38 or newer. Older Git reports it as an unknown option,
    /// which is surfaced as a workspace-specific error rather than a generic
    /// command failure.
    fn merge_tree(&self, ours: &CommitId, theirs: &CommitId) -> Result<CommitId> {
        let mut cmd = self.git_plumbing();
        cmd.arg("merge-tree")
            .arg("--write-tree")
            .arg("--merge-base")
            .arg(merge_base_of(self, ours, theirs))
            .arg(ours.as_ref())
            .arg(theirs.as_ref());

        match run_git_capture(cmd, "git merge-tree") {
            Ok(out) => Ok(CommitId(
                out.lines().next().unwrap_or_default().trim().into(),
            )),
            // A conflicted merge exits non-zero, so `run_git_capture` reports it
            // as a command failure. The tree it wanted to write is not usable,
            // and the caller names the branches involved.
            Err(error) => Err(Error::new(ErrorKind::Backend(format!(
                "the two branches conflict and cannot both be applied: {error}"
            )))),
        }
    }

    /// Create a commit from `tree` with the given parents.
    fn commit_tree(&self, tree: &CommitId, parents: &[&CommitId], message: &str) -> Result<CommitId> {
        let mut cmd = self.git_plumbing();
        cmd.arg("commit-tree").arg(tree.as_ref());
        for parent in parents {
            let parent: &str = parent.as_ref();
            cmd.arg("-p").arg(parent);
        }
        cmd.arg("-m").arg(message);
        let id = run_git_capture(cmd, "git commit-tree")?;
        Ok(CommitId(id.trim().into()))
    }

    /// Refuse operations that would disturb a dirty index.
    fn require_clean_index(&self) -> Result<()> {
        let mut cmd = self.git_plumbing();
        cmd.arg("diff-index").arg("--quiet").arg("HEAD").arg("--");
        match run_git_capture_bytes(cmd, "git diff-index") {
            // `diff-index --quiet` exits 1 when the index differs from HEAD,
            // and `run_git_capture` turns that into an error carrying git's
            // stderr — which is empty for a clean result. Either way the
            // message below is the one the user needs.
            Ok(_) => Ok(()),
            Err(_) => Err(Error::new(ErrorKind::Backend(
                "the index has staged changes; commit or unstage them first".to_string(),
            ))),
        }
    }

    pub(super) fn virtual_branch_push_target_impl(
        &self,
        name: &str,
        workspace: &WorkspaceState,
    ) -> Option<String> {
        let branch: &VirtualBranch = workspace.get(name)?;
        Some(branch.base_branch(&workspace.target).to_string())
    }

    pub(super) fn push_virtual_branch_impl(&self, name: &str) -> Result<CommandOutput> {
        crate::util::validate_ref_like_arg(name, "branch name")?;
        let Some(remote) = self.preferred_remote_name()? else {
            return Err(Error::new(ErrorKind::Backend(
                "this repository has no remote to push to".to_string(),
            )));
        };

        // The named ref on both sides, not `HEAD`: in a workspace the user is
        // usually sitting on an unrelated branch, and pushing `HEAD` would
        // publish that one instead of the branch they clicked.
        let label = format!("git push --set-upstream {remote} {name}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--set-upstream")
            .arg("--")
            .arg(&remote)
            .arg(format!("refs/heads/{name}:refs/heads/{name}"));
        crate::util::run_git_simple(cmd, &label)?;
        Ok(CommandOutput::empty_success(label))
    }
}

/// The merge base of two commits, or `ours` when they share no history.
///
/// `git merge-tree` needs an explicit base. Falling back to `ours` gives a
/// two-way merge, which is the best available answer for unrelated histories
/// and surfaces conflicts rather than silently picking a side.
fn merge_base_of(repo: &GixRepo, ours: &CommitId, theirs: &CommitId) -> String {
    let mut cmd = repo.git_plumbing();
    cmd.arg("merge-base")
        .arg(ours.as_ref())
        .arg(theirs.as_ref());
    run_git_capture(cmd, "git merge-base")
        .map(|base| base.trim().to_string())
        .ok()
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| ours.to_string())
}

/// Write via a temporary file and rename, so a crash mid-write cannot leave a
/// truncated workspace file that reads as an empty workspace.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "writing {}: {e}",
            temp.display()
        )))
    })?;
    std::fs::rename(&temp, path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "replacing {}: {e}",
            path.display()
        )))
    })
}

/// Whether `name` is one of GitComet's own branches rather than a user's.
/// The mode out of `git ls-files -s` or `git ls-tree`, if the output names one.
///
/// Only the two executable modes are interesting; anything else is a plain file
/// and staging it as `100644` says the same thing.
fn parse_mode(output: &str) -> Option<&'static str> {
    match output.split_whitespace().next()? {
        "100755" => Some("100755"),
        "100644" => Some("100644"),
        _ => None,
    }
}

pub(crate) fn is_workspace_branch(name: &str) -> bool {
    name.starts_with(WORKSPACE_REF_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_branches_are_recognized_by_their_prefix() {
        assert!(is_workspace_branch(WORKSPACE_BRANCH));
        assert!(is_workspace_branch("gitcomet/anything"));
        assert!(!is_workspace_branch("feature/api"));
        assert!(!is_workspace_branch("main"));
    }

    #[test]
    fn the_state_file_sits_inside_the_workspace_directory() {
        let dir = PathBuf::new("/repo/.git").join(WORKSPACE_DIR);
        assert!(dir.ends_with(WORKSPACE_DIR));
        assert_eq!(dir.join(WORKSPACE_STATE_FILE).file_name().unwrap(), "workspace.json");
    }
}
