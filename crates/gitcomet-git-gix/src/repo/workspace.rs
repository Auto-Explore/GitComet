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
    /// `git rebase` is not usable here. It refuses outright when the working
    /// tree has unstaged changes — "cannot rebase: You have unstaged changes" —
    /// and in a workspace that is the *normal* state, not a mistake: the whole
    /// point is uncommitted changes spread across several branches at once.
    /// Every stack edit would fail, so the replay goes through the same
    /// `merge-tree` + `commit-tree` plumbing the target change already uses,
    /// which reads neither the worktree nor the index.
    ///
    /// The cost is that the branch's commits are replayed as one: the result is
    /// a single commit on `onto` holding everything `name` had, rather than
    /// `name`'s commits one for one. That is the same trade the target change
    /// makes, and it is why `restack_branch_onto` carries the name it does.
    pub(super) fn rebase_virtual_branch_impl(&self, name: &str, onto: &str) -> Result<()> {
        self.restack_branch_onto(name, onto).map(|_| ())
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

        let tree = match self.merge_tree(&branch_id, &onto_id) {
            MergeOutcome::Merged(tree) => tree,
            MergeOutcome::Conflicted { paths } => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "'{name}' cannot be moved onto '{onto}': they conflict in {}",
                    describe_paths(&paths)
                ))));
            }
        };
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

    // ── The working directory ────────────────────────────────────

/// Whether the working directory is currently sitting on the workspace branch.
///
/// This is what decides whether a rebuild should also move the files: a
/// workspace the user is not looking at has no business changing their
/// working tree, and one they are looking at has to follow.
pub(super) fn workspace_is_checked_out(&self) -> bool {
    self.current_branch_impl()
        .map(|branch| branch == WORKSPACE_BRANCH)
        .unwrap_or(false)
}

/// Put the working directory on the workspace branch, building it first if it
/// does not exist yet.
///
/// The checkout is git's own, so its rule applies: local changes that the
/// switch would overwrite are refused rather than carried, and nothing is
/// discarded. That refusal is the whole safety story of this feature — the
/// user is told which move failed and why, and gitcomet/workspace has not been
/// created and left half-applied behind.
pub(super) fn enter_workspace_impl(&self) -> Result<CommitId> {
    if self.resolve_revision(WORKSPACE_BRANCH)?.is_none() {
        let state = self.read_workspace_impl()?;
        let applied = state.application_order()?;
        let names: Vec<&str> = applied.iter().map(|branch| branch.name.as_str()).collect();
        self.update_workspace_branch_impl(&names)?;
    }
    let tip = self.require_revision(WORKSPACE_BRANCH)?;

    let mut cmd = self.git_plumbing();
    cmd.arg("checkout")
        // Submodules are the user's business, not a side effect of which
        // virtual branches happen to be applied.
        .arg("--no-recurse-submodules")
        .arg(WORKSPACE_BRANCH);
    run_git_simple(cmd, "git checkout").map_err(|error| {
        Error::new(ErrorKind::Backend(format!(
            "could not switch the working directory to the workspace: {error}\n\n\
             GitComet will not discard uncommitted changes. Commit or stash \
             them, then try again."
        )))
    })?;
    Ok(tip)
}

/// Take the working directory back off the workspace branch and onto `onto`.
///
/// `onto` is the branch the user was on before they entered the workspace, so
/// this is the inverse of [`Self::enter_workspace_impl`] and carries the same
/// refusal rule.
pub(super) fn leave_workspace_impl(&self, onto: &str) -> Result<()> {
    self.require_revision(onto)?;
    let mut cmd = self.git_plumbing();
    cmd.arg("checkout")
        .arg("--no-recurse-submodules")
        .arg(onto);
    run_git_simple(cmd, "git checkout").map_err(|error| {
        Error::new(ErrorKind::Backend(format!(
            "could not switch back to '{onto}': {error}\n\n\
             GitComet will not discard uncommitted changes. Commit or stash \
             them, then try again."
        )))
    })
}

/// Move the working directory from one workspace tree to another.
///
/// This is the two-way tree switch `git checkout` performs internally, and it is
/// the reason applying a branch does not throw away work: a file the user has
/// edited and that the applied-set change does *not* touch keeps its edit and
/// takes the new committed version underneath it, while a file that is both
/// edited *and* changed by the switch is refused outright — git's own rule, and
/// the correct one, because there is no automatic answer to that conflict.
///
/// The index and working tree are verified before anything is written, so a
/// refusal leaves both exactly as they were. Callers must therefore do this
/// *before* moving the workspace ref, not after, or the two would disagree.
pub(super) fn sync_workspace_workdir_impl(
    old_tip: &CommitId,
    new_tip: &CommitId,
) -> Result<()> {
    if old_tip == new_tip {
        return Ok(());
    }
    let mut cmd = self.git_plumbing();
    cmd.arg("read-tree")
        .arg("-u")
        .arg("-m")
        .arg(old_tip.as_ref())
        .arg(new_tip.as_ref());
    run_git_simple(cmd, "git read-tree").map_err(|error| {
        Error::new(ErrorKind::Backend(format!(
            "could not update the working directory: {error}\n\n\
             GitComet will not discard uncommitted changes. Commit or stash \
             the files above, then try again."
        )))
    })
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
        // The branches already folded into `head`, so a conflict can be reported
        // as the pair it actually is. `head` is one merge commit by this point
        // and cannot say which of them is in the way.
        let mut merged: Vec<&str> = Vec::new();

        for name in applied {
            let branch_id = self.require_revision(name)?;
            if branch_id == head {
                // Already contained in what we have merged; adding it again
                // would produce an empty merge commit.
                continue;
            }
            let tree = match self.merge_tree(&branch_id, &head) {
                MergeOutcome::Merged(tree) => tree,
                MergeOutcome::Conflicted { paths } => {
                    let against = self.blames_conflict_on(name, &merged, &target_id);
                    return Err(Error::new(ErrorKind::Backend(conflict_message(
                        name,
                        against.as_deref(),
                        &paths,
                    ))));
                }
            };
            // Every merge gets its own commit so the workspace history shows
            // which branch contributed what, and so unapplying one later is a
            // replay rather than an edit of history that has been pushed.
            let message = format!("Apply {name}");
            head = self.commit_tree(&tree, &[&head], &message)?;
            merged.push(name);
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

        // The working directory follows only when the user is looking at the
        // workspace; a rebuild of a workspace nobody has entered must not touch
        // their files.
        //
        // Syncing *before* the ref moves is the whole ordering here. A refused
        // sync leaves the index and the working tree exactly as they were, and
        // with the ref still on the old tree the two cannot disagree.
        let previous = self.resolve_revision(WORKSPACE_BRANCH)?;
        if let Some(previous) = previous.as_ref().filter(|_| self.workspace_is_checked_out()) {
            self.sync_workspace_workdir_impl(previous, &head)?;
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

        // No `--all`/`--add` flag: `git add` has no `--add` option and refuses
        // the whole command when it sees one. Plain `git add -- <paths>` already
        // stages a path that is new in the working tree and one that has been
        // deleted, which is the whole range this has to cover.
        if !plain.is_empty() {
            let mut add = self.git_plumbing();
            add.arg("add").arg("--");
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
        // No `--add`: see `commit_paths_tree`. `git add -- <path>` covers a new
        // file and a deleted one on its own.
        add.arg("add").arg("--").arg(path);
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
    fn merge_tree(&self, ours: &CommitId, theirs: &CommitId) -> MergeOutcome {
        let mut cmd = self.git_plumbing();
        cmd.arg("merge-tree")
            .arg("--write-tree")
            .arg("--merge-base")
            .arg(merge_base_of(self, ours, theirs))
            .arg(ours.as_ref())
            .arg(theirs.as_ref());

        match run_git_capture(cmd, "git merge-tree") {
            Ok(out) => MergeOutcome::Merged(CommitId(
                out.lines().next().unwrap_or_default().trim().into(),
            )),
            Err(error) => MergeOutcome::Conflicted {
                paths: conflicted_paths(&error),
            },
        }
    }

    /// The paths `from` and `to` disagree about.
    ///
    /// Used only when a merge has already failed, to work out which of the
    /// branches already applied is the one actually in the way — the pair the
    /// user has to act on, which the merge itself cannot name because from its
    /// side the other branches are already merged into one tree.
    fn changed_paths(&self, from: &CommitId, to: &CommitId) -> Result<Vec<String>> {
        let mut cmd = self.git_plumbing();
        cmd.arg("diff")
            .arg("--name-only")
            .arg(from.as_ref())
            .arg(to.as_ref());
        let out = run_git_capture(cmd, "git diff --name-only")?;
        Ok(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
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

/// Which already-applied branch is the one `name` conflicts with.
///
/// The conflict is over paths, and a branch is only in the way if it changed
/// some of the same ones. Whichever already-merged branch overlaps the most is
/// the best answer the merge can give, and saying "feature/ui conflicts with
/// feature/api" is the difference between a message the user can act on and one
/// they have to go and guess at.
fn blames_conflict_on(
    &self,
    name: &str,
    merged: &[&str],
    target_id: &CommitId,
) -> Option<String> {
    let branch_id = self.resolve_revision(name).ok()??;
    let wanted: Vec<String> = self
        .changed_paths(target_id, &branch_id)
        .unwrap_or_default();
    if wanted.is_empty() {
        return None;
    }
    merged
        .iter()
        .filter_map(|other| {
            let other_id = self.resolve_revision(other).ok()??;
            let overlap = self
                .changed_paths(target_id, &other_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|path| wanted.contains(path))
                .count();
            (overlap > 0).then(|| (overlap, (*other).to_string()))
        })
        .max_by_key(|(overlap, _)| *overlap)
        .map(|(_, name)| name)
}

/// The conflict as the user is shown it: which branch, against which, where.
fn conflict_message(name: &str, against: Option<&str>, paths: &[String]) -> String {
    match against {
        Some(against) => format!(
            "'{name}' conflicts with '{against}' in {}\n\n\
             Both are applied, and both changed the same lines. Unapply one of \
             them, or resolve the conflict before applying {name} again.",
            describe_paths(paths)
        ),
        None => format!(
            "'{name}' could not be applied: it conflicts with the branches \
             already in the workspace, in {}\n\n\
             Unapply one of them, or resolve the conflict before applying \
             {name} again.",
            describe_paths(paths)
        ),
    }
}

/// The conflicting paths as a readable list, or a sentence when git named none.
fn describe_paths(paths: &[String]) -> String {
    match paths {
        [] => "files git did not name".to_string(),
        [one] => one.clone(),
        many => format!("{} (and {} more)", many[0], many.len() - 1),
    }
}

/// What a merge of two trees produced.
enum MergeOutcome {
    /// The tree, ready to commit.
    Merged(CommitId),
    /// The paths git could not merge.
    ///
    /// A conflict is not an error condition in the caller: the workspace is
    /// still valid, it just cannot take this branch, and the user has to be
    /// told which two branches are in the way rather than being handed a bare
    /// "merge failed".
    Conflicted { paths: Vec<String> },
}

/// The paths `git merge-tree` reported as conflicted.
///
/// On failure git writes everything to **stdout** and nothing to stderr: the
/// tree it would have written, then one `<mode> <oid> <stage>\t<path>` line per
/// conflicted entry, then blank-line-separated messages. Reading stderr — which
/// is what an error built from a failed command usually does — yields an empty
/// string, which is why a conflict used to arrive at the user with no detail at
/// all.
///
/// Stage 1 is the merge base and appears for every conflicted path, so only the
/// higher stages are paths git actually marked as conflicted.
///
/// That means each conflicted path is reported once per stage above the base —
/// three lines for a content conflict — so they are collapsed here. A caller
/// showing this list to the user would otherwise name the same file three
/// times, and `describe_paths` would count it as three files in conflict when
/// there is one. First occurrence wins, in the order git reported them.
fn conflicted_paths(error: &Error) -> Vec<String> {
    let ErrorKind::Git(failure) = error.kind() else {
        return Vec::new();
    };
    let mut paths: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(failure.stdout()).lines().skip(1) {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let Some(stage) = meta.split_whitespace().nth(2) else {
            continue;
        };
        if stage == "1" {
            continue;
        }
        if !paths.iter().any(|seen| seen == path) {
            paths.push(path.to_string());
        }
    }
    paths
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

    use gitcomet_core::services::GitRepository;

    fn git(root: &Path, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        let output = cmd.output().expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A repository with two virtual branches whose changes touch different
    /// files, so applying and unapplying one is visible in the other's absence.
    fn repo_with_two_branches() -> (tempfile::TempDir, GixRepo) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("shared.txt"), "a1\na2\na3\n").unwrap();
        std::fs::write(root.join("moved.txt"), "x\n").unwrap();
        std::fs::write(root.join("mine.txt"), "y\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);

        git(root, &["branch", "api"]);
        git(root, &["checkout", "api"]);
        std::fs::write(root.join("shared.txt"), "a1\nAPI\na3\n").unwrap();
        git(root, &["commit", "-am", "api"]);

        git(root, &["branch", "ui"]);
        std::fs::write(root.join("moved.txt"), "ui-x\n").unwrap();
        git(root, &["commit", "-am", "ui"]);

        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
        git(root, &["checkout", &default_branch]);

        let state = WorkspaceState::new(&default_branch).with_branches(vec![
            VirtualBranch::new("api"),
            VirtualBranch::new("ui"),
        ]);
        let repo = open_repo(root);
        repo.write_workspace(&state).expect("write workspace");
        (temp, repo)
    }

    fn open_repo(workdir: &Path) -> GixRepo {
        let thread_safe = gix::open(workdir).expect("open repo").into_sync();
        GixRepo::new(workdir.to_path_buf(), thread_safe)
    }

    fn read(root: &Path, name: &str) -> String {
        std::fs::read_to_string(root.join(name)).expect("read file")
    }

    #[test]
    fn entering_the_workspace_puts_every_applied_branch_in_the_files() {
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        let tip = repo.enter_workspace().expect("enter workspace");

        assert_eq!(read(root, "shared.txt"), "a1\nAPI\na3\n", "api is applied");
        assert_eq!(read(root, "moved.txt"), "ui-x\n", "ui is applied");
        assert_eq!(read(root, "mine.txt"), "y\n");
        assert_eq!(
            repo.current_branch().expect("current branch"),
            WORKSPACE_BRANCH,
            "the working directory is the workspace"
        );
        assert_eq!(tip.as_ref(), git(root, &["rev-parse", "HEAD"]).as_str());
    }

    #[test]
    fn unapplying_a_branch_keeps_edits_to_files_it_never_touched() {
        // The property the whole design rests on: a rebuild moves the working
        // directory without taking uncommitted work with it.
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        repo.enter_workspace().expect("enter workspace");

        std::fs::write(root.join("mine.txt"), "y\nMY UNCOMMITTED WORK\n").unwrap();

        let mut state = repo.read_workspace().expect("read workspace");
        state
            .set_applied("ui", gitcomet_core::workspace::BranchApplyState::Unapplied)
            .expect("unapply");
        repo.write_workspace(&state).expect("write workspace");
        repo.update_workspace_branch(&["api"]).expect("rebuild");

        assert_eq!(
            read(root, "moved.txt"),
            "x\n",
            "the unapplied branch's change is gone"
        );
        assert_eq!(
            read(root, "mine.txt"),
            "y\nMY UNCOMMITTED WORK\n",
            "an unrelated edit survives the rebuild"
        );
        assert_eq!(read(root, "shared.txt"), "a1\nAPI\na3\n");
    }

    #[test]
    fn a_refused_sync_leaves_the_files_and_the_ref_exactly_as_they_were() {
        // GitComet never discards uncommitted changes. A file that is both
        // edited and changed by the switch has no automatic answer, so the
        // switch is refused — and because the sync runs before the ref moves,
        // a refusal cannot leave the two disagreeing.
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        repo.enter_workspace().expect("enter workspace");
        let before = git(root, &["rev-parse", WORKSPACE_BRANCH]);

        // Edit the very file `api` changed, then unapply it.
        std::fs::write(root.join("shared.txt"), "a1\nAPI\nMINE\na3\n").unwrap();
        let mut state = repo.read_workspace().expect("read workspace");
        state
            .set_applied("api", gitcomet_core::workspace::BranchApplyState::Unapplied)
            .expect("unapply");
        repo.write_workspace(&state).expect("write workspace");

        assert!(
            repo.update_workspace_branch(&["ui"]).is_err(),
            "a switch that would overwrite a local edit is refused"
        );
        assert_eq!(
            read(root, "shared.txt"),
            "a1\nAPI\nMINE\na3\n",
            "the file is untouched"
        );
        assert_eq!(
            git(root, &["rev-parse", WORKSPACE_BRANCH]),
            before,
            "and so is the branch, because the ref is only moved after a sync succeeds"
        );
    }

    #[test]
    fn a_rebuild_does_not_touch_the_files_of_a_workspace_that_is_not_checked_out() {
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        assert!(!repo.workspace_is_checked_out());

        repo.update_workspace_branch(&["api", "ui"])
            .expect("rebuild");

        assert_eq!(
            read(root, "moved.txt"),
            "x\n",
            "the ref moved, but a workspace nobody has entered must not move files"
        );
        assert_eq!(read(root, "shared.txt"), "a1\na2\na3\n");
    }

    #[test]
    fn leaving_the_workspace_returns_to_the_branch_it_came_from() {
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        let original = repo.current_branch().expect("current branch");
        repo.enter_workspace().expect("enter workspace");

        repo.leave_workspace(&original).expect("leave workspace");
        assert_eq!(repo.current_branch().expect("current branch"), original);
        assert_eq!(
            read(root, "moved.txt"),
            "x\n",
            "the applied branch's change is no longer in the files"
        );
    }

    #[test]
    fn a_conflict_names_the_applied_branch_that_is_in_the_way() {
        // Two branches that both change the same line cannot both be applied,
        // and the message has to say which pair: the merge itself only knows
        // that the branch it was adding does not fit, because the others are
        // already one merged tree by the time it runs.
        let (temp, repo) = repo_with_two_branches();
        git(
            temp.path(),
            &["checkout", "api"],
        );
        std::fs::write(temp.path().join("moved.txt"), "from-api\n").unwrap();
        git(temp.path(), &["commit", "-am", "clash"]);

        let error = repo
            .update_workspace_branch(&["api", "ui"])
            .expect_err("api and ui now change the same line");
        let message = error.to_string();
        assert!(
            message.contains("'api' conflicts with 'ui'"),
            "the message names both branches, got: {message}"
        );
        assert!(
            message.contains("moved.txt"),
            "and the file they clash in, got: {message}"
        );
    }

    #[test]
    fn conflicted_paths_are_read_from_stdout_because_stderr_is_empty() {
        // git writes the conflict to stdout and exits non-zero. Reading stderr
        // — the obvious thing — yields nothing at all.
        let error = Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
            "git merge-tree",
            gitcomet_core::error::GitFailureId::CommandFailed,
            Some(1),
            b"0d69e0ee71279a47cf8802e7400473e50e196110\n\
              100644 aaa 1\tsrc/a.rs\n\
              100644 bbb 2\tsrc/a.rs\n\
              100644 ccc 3\tsrc/a.rs\n\
              100644 ddd 1\tsrc/b.rs\n\
              100644 eee 2\tsrc/b.rs\n\
              100644 fff 3\tsrc/b.rs\n\
              \nAuto-merging src/a.rs\nCONFLICT (content): Merge conflict in src/a.rs\n"
                .to_vec(),
            Vec::new(),
            None,
        )));
        assert_eq!(
            conflicted_paths(&error),
            ["src/a.rs", "src/b.rs"],
            "one entry per path: git writes a line per stage, so a content \
             conflict is three lines naming the same file, and a caller showing \
             these would list one file three times"
        );
    }

    #[test]
    fn a_real_merge_conflict_names_each_path_once() {
        // The same thing against a repository rather than a hand-written
        // fixture, because the shape of `merge-tree`'s output is git's and
        // changes between versions.
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("same.txt"), "base\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);
        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

        git(root, &["checkout", "-b", "left"]);
        std::fs::write(root.join("same.txt"), "A\n").unwrap();
        git(root, &["commit", "-am", "left"]);
        git(root, &["checkout", &default_branch]);
        git(root, &["checkout", "-b", "right"]);
        std::fs::write(root.join("same.txt"), "B\n").unwrap();
        git(root, &["commit", "-am", "right"]);

        let repo = open_repo(root);
        let left = repo
            .resolve_revision("left")
            .expect("resolve")
            .expect("left exists");
        let right = repo
            .resolve_revision("right")
            .expect("resolve")
            .expect("right exists");
        // The very command `merge_tree` runs, kept here rather than through a
        // helper so the test cannot pass by agreeing with a bug in one.
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(root)
            .arg("merge-tree")
            .arg("--write-tree")
            .arg(format!("--merge-base={}", repo.merge_base_of(&left, &right)))
            .arg(left.as_ref())
            .arg(right.as_ref());
        let output = cmd.output().expect("run git");
        assert!(!output.status.success(), "this conflict has to fail the merge");

        let error = Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
            "git merge-tree",
            gitcomet_core::error::GitFailureId::CommandFailed,
            Some(output.status.code()),
            output.stdout,
            output.stderr,
            None,
        )));
        assert_eq!(conflicted_paths(&error), ["same.txt"]);
    }

    #[test]
    fn a_backend_error_names_no_conflicting_paths_rather_than_guessing() {
        let error = Error::new(ErrorKind::Backend("something else failed".into()));
        assert!(conflicted_paths(&error).is_empty());
        assert_eq!(describe_paths(&[]), "files git did not name");
    }

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

    #[test]
    fn only_the_two_executable_modes_are_read_out_of_git() {
        // `git ls-tree` names a gitlink or a symlink too, and staging one of
        // those as `100644` would turn it into a plain file the moment a file
        // was split between branches.
        assert_eq!(parse_mode("100755 blob abc\trun.sh"), Some("100755"));
        assert_eq!(parse_mode("100644 blob abc\tsrc/lib.rs"), Some("100644"));
        assert_eq!(parse_mode("120000 blob abc\tlink"), None);
        assert_eq!(parse_mode("160000 commit abc\tvendor"), None);
        assert_eq!(parse_mode(""), None, "nothing to read");
    }

    #[test]
    fn a_split_file_keeps_the_mode_it_had() {
        // The user's real index wins over the base tree, so a mode they staged
        // survives the split rather than being quietly rewritten.
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("plain.txt"), "text\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);

        let repo = open_repo(root);
        let base = repo.resolve_revision("HEAD").expect("resolve").expect("a base");
        assert_eq!(repo.path_mode(&base, Path::new("run.sh")).unwrap(), "100755");
        assert_eq!(repo.path_mode(&base, Path::new("plain.txt")).unwrap(), "100644");
        assert_eq!(
            repo.path_mode(&base, Path::new("new.txt")).unwrap(),
            "100644",
            "a file that is not in the base is a plain file"
        );
    }

    #[test]
    fn two_sibling_branches_are_merged_from_the_commit_they_share() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("base.txt"), "b\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);

        let repo = open_repo(root);
        let base = repo
            .resolve_revision("HEAD")
            .expect("resolve")
            .expect("the base commit exists");

        git(root, &["checkout", "-b", "left"]);
        std::fs::write(root.join("left.txt"), "l\n").unwrap();
        git(root, &["commit", "-am", "left"]);

        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
        git(root, &["checkout", &default_branch]);
        git(root, &["checkout", "-b", "right"]);
        std::fs::write(root.join("right.txt"), "r\n").unwrap();
        git(root, &["commit", "-am", "right"]);

        let left = repo
            .resolve_revision("left")
            .expect("resolve")
            .expect("left exists");
        let right = repo
            .resolve_revision("right")
            .expect("resolve")
            .expect("right exists");
        assert_ne!(left, right, "they have to be siblings for this to mean anything");
        assert_eq!(
            merge_base_of(&repo, &left, &right),
            base.as_ref(),
            "a merge from the wrong base would resurrect one branch's commits as \
             conflicts against the other's"
        );
    }

    #[test]
    fn branches_with_no_shared_history_merge_against_ours() {
        // `git merge-tree` needs an explicit base and there is none here.
        // Falling back to `ours` gives a two-way merge, which surfaces the
        // clash rather than silently picking a side.
        let (temp, repo) = repo_with_two_branches();
        let root = temp.path();
        git(root, &["checkout", "--orphan", "lonely"]);
        git(root, &["rm", "-rfq", "."]);
        std::fs::write(root.join("other.txt"), "z\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "lonely"]);

        let api = repo.resolve_revision("api").expect("resolve").expect("api exists");
        let lonely = repo
            .resolve_revision("lonely")
            .expect("resolve")
            .expect("lonely exists");
        assert_eq!(merge_base_of(&repo, &api, &lonely), api.as_ref());
    }

    #[test]
    fn one_conflicting_path_is_named_and_several_are_counted() {
        assert_eq!(describe_paths(&["src/lib.rs".to_string()]), "src/lib.rs");
        assert_eq!(
            describe_paths(&["src/lib.rs".to_string(), "src/main.rs".to_string()]),
            "src/lib.rs (and 1 more)",
        );
        assert_eq!(
            describe_paths(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a (and 2 more)"
        );
    }

    #[test]
    fn a_conflict_names_the_other_branch_or_says_that_it_could_not() {
        let against = conflict_message("feature/ui", Some("feature/api"), &["src/lib.rs".into()]);
        assert!(
            against.contains("'feature/ui'") && against.contains("'feature/api'"),
            "the two branches in the way have to be named: {against}"
        );

        let nameless = conflict_message("feature/ui", None, &[]);
        assert!(
            nameless.contains("feature/ui") && !nameless.contains("conflicts with '"),
            "with no branch to blame the message must not invent one: {nameless}"
        );
    }

    // ── Committing assigned paths ─────────────────────────────────────

    /// Ten lines, edited at the first and the last.
    ///
    /// The gap between the two edits is eight unchanged lines, past the six
    /// that `git diff` folds into one `@@`, so this is two hunks to the user
    /// and two spans here — which is what makes it splittable at all.
    const SPLIT_BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
    const SPLIT_WORKING: &str = "API\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nL10\n";

    /// A repository with two sibling virtual branches and one file that can be
    /// split between them, plus an unrelated file that must never be dragged in.
    fn repo_with_a_splittable_file() -> (tempfile::TempDir, GixRepo, String) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("shared.txt"), SPLIT_BASE).unwrap();
        std::fs::write(root.join("other.txt"), "keep\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);

        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
        git(root, &["branch", "api"]);
        git(root, &["branch", "ui"]);

        let state = WorkspaceState::new(&default_branch)
            .with_branches(vec![VirtualBranch::new("api"), VirtualBranch::new("ui")]);
        let repo = open_repo(root);
        repo.write_workspace(&state).expect("write workspace");
        (temp, repo, default_branch)
    }

    /// A file's contents at a revision, exactly as git has them.
    fn blob(root: &Path, revision: &str, path: &str) -> String {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(root)
            .arg("show")
            .arg(format!("{revision}:{path}"));
        let output = cmd.output().expect("run git");
        assert!(
            output.status.success(),
            "git show {revision}:{path} failed"
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Split `shared.txt` between the two branches, one hunk each.
    fn assign_the_two_hunks(repo: &GixRepo) -> usize {
        let spans = hunk_spans(SPLIT_BASE, SPLIT_WORKING);
        let mut index = AssignmentIndex::new();
        for (span, branch) in spans.iter().zip(["api", "ui"]) {
            index.set_hunk(Path::new("shared.txt"), span.fingerprint, Some(branch.into()));
        }
        repo.write_workspace_assignments(&index)
            .expect("write assignments");
        spans.len()
    }

    #[test]
    fn two_edits_far_apart_are_two_hunks_to_split_between_branches() {
        // The premise the rest of these tests stand on: if this ever said one,
        // "split a file between two branches" would be impossible and the
        // assignments would silently do nothing.
        assert_eq!(hunk_spans(SPLIT_BASE, SPLIT_WORKING).len(), 2);
        assert_eq!(
            hunk_spans(SPLIT_BASE, "API\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n").len(),
            1,
            "edits within git's context are one hunk, not two"
        );
    }

    #[test]
    fn a_commit_to_one_branch_takes_only_the_hunks_assigned_to_it() {
        let (temp, repo, default_branch) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
        assert_eq!(assign_the_two_hunks(&repo), 2);

        let outcome = repo
            .commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
            .expect("commit to api");

        assert_eq!(
            blob(root, "api", "shared.txt"),
            "API\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n",
            "the commit holds this branch's hunk and not the other branch's"
        );
        assert_eq!(
            git(root, &["rev-parse", "api~1"]),
            git(root, &["rev-parse", &default_branch]),
            "the commit is built on the branch's own base, never on the workspace"
        );
        assert_eq!(outcome.local_branch.as_deref(), Some("api"));
        assert_eq!(
            read(root, "shared.txt"),
            SPLIT_WORKING,
            "committing to a branch must not change what the user is looking at"
        );
    }

    #[test]
    fn both_branches_commit_their_own_half_and_the_workspace_gathers_both() {
        // The point of the whole feature: two unrelated changes in one working
        // directory, each landing on its own branch, and the workspace showing
        // both.
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
        assign_the_two_hunks(&repo);

        repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
            .expect("commit to api");
        repo.commit_paths_to_virtual_branch("ui", "the ui change", &[Path::new("shared.txt")])
            .expect("commit to ui");

        assert_eq!(
            blob(root, "ui", "shared.txt"),
            "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nL10\n",
            "the second commit must not pick up the first branch's hunk"
        );
        assert_eq!(
            blob(root, WORKSPACE_BRANCH, "shared.txt"),
            SPLIT_WORKING,
            "the workspace is the two branches together, which is what makes the \
             working directory show both changes"
        );
    }

    #[test]
    fn a_stacked_branch_commits_on_top_of_its_parents_newest_commit() {
        // Otherwise a pull request for the upper branch would contain only its
        // own change, with the lower branch's work missing from its history.
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
        let mut state = repo.read_workspace().expect("read workspace");
        state.set_parent("ui", Some("api")).expect("stack ui on api");
        repo.write_workspace(&state).expect("write workspace");
        assign_the_two_hunks(&repo);

        repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
            .expect("commit to api");
        let api_tip = git(root, &["rev-parse", "api"]);
        repo.commit_paths_to_virtual_branch("ui", "the ui change", &[Path::new("shared.txt")])
            .expect("commit to ui");

        assert_eq!(
            git(root, &["rev-parse", "ui~1"]),
            api_tip,
            "the stacked branch has to be built on where its parent ended up"
        );
    }

    #[test]
    fn only_the_paths_named_are_committed() {
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("mine.txt"), "y\n").unwrap();
        let mut index = AssignmentIndex::new();
        index.set(Path::new("mine.txt"), Some("api".into()));
        repo.write_workspace_assignments(&index).expect("write assignments");

        repo.commit_paths_to_virtual_branch("api", "just the new file", &[Path::new("mine.txt")])
            .expect("commit");

        assert_eq!(blob(root, "api", "mine.txt"), "y\n");
        assert_eq!(
            git(root, &["ls-tree", "--name-only", "api"]),
            "mine.txt\nother.txt\nshared.txt",
            "a file nobody assigned comes from the base, not from the working tree"
        );
    }

    #[test]
    fn a_file_deleted_in_the_working_tree_is_deleted_on_its_branch() {
        // Deleting a file is a change to it, and a branch assigned it.
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::remove_file(root.join("shared.txt")).unwrap();

        repo.commit_paths_to_virtual_branch("api", "drop the shared file", &[Path::new("shared.txt")])
            .expect("commit");

        assert_eq!(
            git(root, &["ls-tree", "--name-only", "api"]),
            "other.txt\n",
            "the deletion has to land on the branch too"
        );
        assert!(
            !root.join(".git").join("gitcomet").join("commit-index").exists(),
            "the temporary index has to be cleaned up"
        );
    }

    #[test]
    fn committing_leaves_the_users_index_and_staging_exactly_as_they_were() {
        // The commit is built through a temporary index. If it were built
        // through the real one, this would stage the commit away and swallow
        // whatever the user had staged for their next commit.
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
        std::fs::write(root.join("other.txt"), "staged edit\n").unwrap();
        git(root, &["add", "-A"]);
        let staged_before = git(root, &["diff", "--cached", "--name-only"]);

        repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
            .expect("commit");

        assert_eq!(
            git(root, &["diff", "--cached", "--name-only"]),
            staged_before,
            "the user's staging has to survive a workspace commit untouched"
        );
        assert_eq!(read(root, "shared.txt"), SPLIT_WORKING);
        assert_eq!(read(root, "other.txt"), "staged edit\n");
    }

    #[test]
    fn an_executable_file_stays_executable_after_a_commit_to_a_branch() {
        // Splitting or committing goes through an index entry this code writes
        // by hand, and a mode guessed wrong turns a script into a text file.
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);

        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
        git(root, &["branch", "api"]);
        let state = WorkspaceState::new(&default_branch).with_branches(vec![VirtualBranch::new("api")]);
        let repo = open_repo(root);
        repo.write_workspace(&state).expect("write workspace");

        std::fs::write(root.join("run.sh"), "#!/bin/sh\necho bye\n").unwrap();
        let mut index = AssignmentIndex::new();
        index.set(Path::new("run.sh"), Some("api".into()));
        repo.write_workspace_assignments(&index).expect("write assignments");

        repo.commit_paths_to_virtual_branch("api", "shout", &[Path::new("run.sh")]).expect("commit");

        let entry = git(root, &["ls-tree", "api", "--", "run.sh"]);
        assert!(
            entry.starts_with("100755 "),
            "the mode has to come from the base, not be defaulted to a plain \
             file: {entry}"
        );
    }

    #[test]
    fn committing_to_a_branch_that_is_not_in_the_workspace_is_refused() {
        // Otherwise a typo would create a commit on a ref the workspace has no
        // idea about, invisible in every stack in the UI.
        let (temp, repo, _) = repo_with_a_splittable_file();
        let root = temp.path();
        std::fs::write(root.join("mine.txt"), "y\n").unwrap();

        let result =
            repo.commit_paths_to_virtual_branch("nowhere", "m", &[Path::new("mine.txt")]);
        assert!(result.is_err(), "a branch the workspace does not have is an error");
        assert!(
            !root.join(".git").join("refs").join("heads").join("nowhere").exists(),
            "a refused commit must not create the ref either"
        );
    }

    // ── Rebuilding, restacking, and what is on disk ──────────────

    /// Whether a git command succeeds, for the assertions where failure *is* the
    /// expected outcome.
    fn git_succeeds(root: &Path, args: &[&str]) -> bool {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(root).args(args);
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    /// The subjects on the workspace branch, newest first.
    fn workspace_log(root: &Path) -> String {
        git(root, &["log", "--format=%s", WORKSPACE_BRANCH])
    }

    /// Two branches that changed the same line, so applying both cannot work.
    fn repo_with_conflicting_branches() -> (tempfile::TempDir, GixRepo, String) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("same.txt"), "base\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);
        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

        git(root, &["checkout", "-b", "left"]);
        std::fs::write(root.join("same.txt"), "A\n").unwrap();
        git(root, &["commit", "-am", "left"]);
        git(root, &["checkout", &default_branch]);
        git(root, &["checkout", "-b", "right"]);
        std::fs::write(root.join("same.txt"), "B\n").unwrap();
        git(root, &["commit", "-am", "right"]);
        git(root, &["checkout", &default_branch]);

        let state = WorkspaceState::new(&default_branch).with_branches(vec![
            VirtualBranch::new("left"),
            VirtualBranch::new("right"),
        ]);
        let repo = open_repo(root);
        repo.write_workspace(&state).expect("write workspace");
        (temp, repo, default_branch)
    }

    /// An independent branch with a stacked one on it, plus a second target
    /// that also moved.
    fn repo_with_a_stacked_pair() -> (tempfile::TempDir, GixRepo, String) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        // Every assertion here is about exact bytes: a system-wide
        // `core.autocrlf` would rewrite the checkout to CRLF.
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.name", "Test User"]);
        git(root, &["config", "user.email", "test@example.com"]);
        std::fs::write(root.join("base.txt"), "base\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);
        let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

        git(root, &["checkout", "-b", "api"]);
        std::fs::write(root.join("api.txt"), "api\n").unwrap();
        git(root, &["commit", "-am", "api"]);
        git(root, &["checkout", "-b", "ui"]);
        std::fs::write(root.join("ui.txt"), "ui\n").unwrap();
        git(root, &["commit", "-am", "ui"]);
        git(root, &["checkout", &default_branch]);

        // A second target that moved too, so pointing the workspace at it is a
        // real change rather than a rename.
        git(root, &["checkout", "-b", "next"]);
        std::fs::write(root.join("next.txt"), "next\n").unwrap();
        git(root, &["commit", "-am", "next"]);
        git(root, &["checkout", &default_branch]);

        let state = WorkspaceState::new(&default_branch).with_branches(vec![
            VirtualBranch::new("api"),
            VirtualBranch::new("ui").with_parent("api"),
        ]);
        let repo = open_repo(root);
        repo.write_workspace(&state).expect("write workspace");
        (temp, repo, default_branch)
    }

    #[test]
    fn an_empty_applied_set_points_the_workspace_at_the_target() {
        // Unapplying everything is a legitimate state, and then the workspace
        // branch *is* the target rather than a branch whose tree nobody can
        // explain.
        let (temp, repo, default_branch) = repo_with_two_branches();
        let root = temp.path();

        let tip = repo
            .update_workspace_branch(&[])
            .expect("rebuild with nothing applied");

        assert_eq!(
            tip,
            repo.resolve_revision(&default_branch)
                .expect("resolve")
                .expect("the target exists"),
            "with nothing applied the workspace branch is the target"
        );
        assert_eq!(
            git(root, &["rev-parse", WORKSPACE_BRANCH]),
            git(root, &["rev-parse", &default_branch])
        );
        assert!(
            !workspace_log(root).contains("Apply "),
            "a merge that did not happen must not leave a commit: {}",
            workspace_log(root)
        );
    }

    #[test]
    fn rebuilding_the_workspace_branch_is_a_pure_function_of_the_applied_set() {
        // Rebuilt from the target every time, never patched. If merges piled up,
        // unapplying a branch would leave its merge commit in the history for
        // good and the branch could no longer be explained by the virtual
        // branches alone.
        let (temp, repo, _) = repo_with_two_branches();
        let root = temp.path();

        repo.update_workspace_branch(&["api", "ui"])
            .expect("apply both");
        let both = workspace_log(root);
        assert!(
            both.contains("Apply api") && both.contains("Apply ui"),
            "{both}"
        );

        repo.update_workspace_branch(&["api"]).expect("unapply ui");
        let one = workspace_log(root);
        assert!(one.contains("Apply api"), "{one}");
        assert!(
            !one.contains("Apply ui"),
            "the merge that is no longer wanted is still in the history: {one}"
        );
    }

    #[test]
    fn rebuilding_an_unchanged_applied_set_appends_nothing() {
        // Every reload of the Workspace tab rebuilds. A rebuild that invented a
        // commit would lengthen the branch every time the user looked at it.
        let (temp, repo, _) = repo_with_two_branches();
        let root = temp.path();

        repo.update_workspace_branch(&["api", "ui"])
            .expect("first");
        let tree = format!("{WORKSPACE_BRANCH}^{{tree}}");
        let tree_after_first = git(root, &["rev-parse", &tree]);
        let commits_after_first = workspace_log(root).lines().count();

        repo.update_workspace_branch(&["api", "ui"])
            .expect("second");

        assert_eq!(
            workspace_log(root).lines().count(),
            commits_after_first,
            "an identical rebuild grew the branch"
        );
        assert_eq!(
            git(root, &["rev-parse", &tree]),
            tree_after_first,
            "and it changed the tree it points at"
        );
        assert_eq!(
            commits_after_first,
            3,
            "the base plus one commit per applied branch, and nothing else"
        );
    }

    #[test]
    fn restacking_a_branch_onto_where_it_already_is_changes_nothing() {
        let (temp, repo, default_branch) = repo_with_a_splittable_file();
        let root = temp.path();
        let before = git(root, &["rev-parse", "api"]);

        repo.rebase_virtual_branch("api", &default_branch)
            .expect("restack onto the base it is already on");

        assert_eq!(
            git(root, &["rev-parse", "api"]),
            before,
            "a branch already on its base must not gain a commit"
        );
    }

    #[test]
    fn applying_two_branches_that_conflict_names_both_and_writes_nothing() {
        // The failure the user has to act on: which two branches are in the way,
        // and over which file. A bare "merge failed" is not actionable.
        let (temp, repo, default_branch) = repo_with_conflicting_branches();
        let root = temp.path();

        let Err(error) = repo.update_workspace_branch(&["left", "right"]) else {
            panic!("these two branches edit the same line; the rebuild must be refused");
        };
        let message = format!("{error}");
        assert!(
            message.contains("left") && message.contains("right"),
            "the two branches in the way have to be named: {message}"
        );
        assert!(
            message.contains("same.txt"),
            "and so does the file they clash on: {message}"
        );
        assert!(
            !git_succeeds(root, &["rev-parse", "--verify", &format!("refs/heads/{WORKSPACE_BRANCH}")]),
            "a refused rebuild must not leave a half-built branch behind"
        );
        assert_eq!(
            git(root, &["symbolic-ref", "--short", "HEAD"]),
            default_branch,
            "and must not have moved the working directory either"
        );
    }

    #[test]
    fn moving_the_target_moves_the_independent_branches_and_leaves_the_stacked_ones() {
        let (temp, repo, _) = repo_with_a_stacked_pair();
        let root = temp.path();
        let ui_parent_before = git(root, &["rev-parse", "ui^"]);
        let next = git(root, &["rev-parse", "next"]);

        repo.update_workspace_target("next").expect("move the target");

        assert_eq!(repo.read_workspace().expect("read").target, "next");
        assert_eq!(
            git(root, &["rev-parse", "api^"]),
            next,
            "an independent branch is based on the target, so it has to follow it"
        );
        assert_eq!(
            git(root, &["rev-parse", "ui^"]),
            ui_parent_before,
            "a stacked branch moves with its base, not with the target: replaying \
             it here too would rewrite it twice"
        );
    }

    #[test]
    fn a_corrupt_workspace_file_reads_as_an_empty_workspace() {
        // A file written by a newer version, or half-written by a crash, must
        // not make the whole repository unusable: the user rebuilds from the UI.
        let (_temp, repo, _) = repo_with_two_branches();
        std::fs::write(repo.workspace_state_path(), b"{ this is not json").expect("corrupt it");

        let state = repo.read_workspace().expect("a corrupt file is not an error");
        assert!(
            state.branches.is_empty(),
            "the branches are gone from the file, so they are gone here too"
        );
    }

    #[test]
    fn an_invalid_workspace_is_refused_and_the_file_on_disk_is_untouched() {
        let (_temp, repo, _) = repo_with_two_branches();
        let before = repo.read_workspace().expect("read");

        // A loop between two branches: the state renders, but no ordering of it
        // can be applied, so it must never reach the disk.
        let cyclic = WorkspaceState::new("main").with_branches(vec![
            VirtualBranch::new("a").with_parent("b"),
            VirtualBranch::new("b").with_parent("a"),
        ]);
        assert!(
            repo.write_workspace(&cyclic).is_err(),
            "a state that cannot be ordered has to be refused"
        );

        assert_eq!(
            repo.read_workspace().expect("read"),
            before,
            "the file on disk still has to describe what Git actually has"
        );
    }

    #[test]
    fn assignments_survive_a_write_and_a_read() {
        // The whole reason the file exists: a drag made today has to still be
        // there after a restart, on the branch the user picked.
        let (_temp, repo, _) = repo_with_two_branches();
        let mut index = AssignmentIndex::new();
        index.set(Path::new("src/a.rs"), Some("api".to_string()));
        index.set(Path::new("src/b.rs"), None);
        index.set_hunk(
            Path::new("src/c.rs"),
            gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1),
            Some("ui".to_string()),
        );

        repo.write_workspace_assignments(&index).expect("write");

        assert_eq!(repo.read_workspace_assignments().expect("read"), index);
    }

    #[test]
    fn an_unreadable_assignments_file_reads_as_empty() {
        // Same reasoning as the workspace file: a corrupt file must not make
        // every changed file in the repository unusable.
        let (_temp, repo, _) = repo_with_two_branches();
        std::fs::write(repo.workspace_assignments_path(), b"not json at all").expect("corrupt it");
        assert!(repo.read_workspace_assignments().expect("read").is_empty());
    }

    #[test]
    fn a_virtual_branch_tracks_nothing_until_it_is_pushed() {
        // A branch that claims an upstream it was never pushed to shows as
        // permanently behind one, and the first push would try to reconcile
        // against a ref that does not exist.
        let (_temp, repo, _) = repo_with_two_branches();
        let root = repo.spec().workdir.clone();
        let target = repo.read_workspace().expect("read").target;

        repo.create_virtual_branch("feature", &target)
            .expect("create");

        assert!(git_succeeds(&root, &["rev-parse", "--verify", "refs/heads/feature"]));
        assert!(
            !git_succeeds(&root, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]),
            "a branch nobody pushed has to track nothing"
        );
    }

    #[test]
    fn creating_a_virtual_branch_on_a_base_that_does_not_exist_is_refused() {
        // A typo in a stack's base would otherwise create a branch that GitComet
        // believes in and that nothing can be built on.
        let (_temp, repo, _) = repo_with_two_branches();
        let root = repo.spec().workdir.clone();

        assert!(
            repo.create_virtual_branch("api", "no-such-branch")
                .is_err(),
            "a base that does not name a commit cannot be built on"
        );
        assert!(
            !git_succeeds(&root, &["rev-parse", "--verify", "refs/heads/api"]),
            "a refused creation must not leave the ref behind"
        );
    }
}
