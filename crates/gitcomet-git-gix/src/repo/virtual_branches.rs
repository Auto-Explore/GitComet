//! The `gitcomet/virtual` branch and the virtual-branch operations behind it.
//!
//! The virtual-branches branch holds the combined tree of every applied virtual
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
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, CommitOperationOutcome, Result};
use gitcomet_core::virtual_branches::{
    AssignmentIndex, VIRTUAL_BRANCHES_BRANCH, VIRTUAL_BRANCHES_REF_PREFIX, VirtualBranch,
    VirtualBranchesState, hunk_spans, synthesize_for_branch,
};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::GixRepo;
use crate::util::{run_git_capture, run_git_capture_bytes, run_git_simple};

/// Directory GitComet's own repository files live in, under the shared
/// `.git` directory so linked worktrees see the same virtual branches.
const VIRTUAL_BRANCHES_DIR: &str = "gitcomet";

const VIRTUAL_BRANCHES_STATE_FILE: &str = "virtual branches.json";
const VIRTUAL_BRANCHES_ASSIGNMENTS_FILE: &str = "assignments.json";

impl GixRepo {
    /// The `.git` directory of the main worktree, where GitComet's own files
    /// live.
    fn virtual_branches_dir(&self) -> PathBuf {
        self.common_dir_impl().join(VIRTUAL_BRANCHES_DIR)
    }

    fn virtual_branches_state_path(&self) -> PathBuf {
        self.virtual_branches_dir()
            .join(VIRTUAL_BRANCHES_STATE_FILE)
    }

    fn virtual_branches_assignments_path(&self) -> PathBuf {
        self.virtual_branches_dir()
            .join(VIRTUAL_BRANCHES_ASSIGNMENTS_FILE)
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

    pub(super) fn read_virtual_branches_impl(&self) -> Result<VirtualBranchesState> {
        let path = self.virtual_branches_state_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // No virtual branches yet is the normal state for a repository that has
            // never opened the Virtual branches view, not an error. Any other failure
            // is reported rather than folded into "empty": saying "empty" would
            // let the next edit overwrite a file the user can still not read,
            // which is the one outcome worse than an error message.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(VirtualBranchesState::default());
            }
            Err(error) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "reading {}: {error}",
                    path.display()
                ))));
            }
        };
        // A virtual branches file written by a newer or hand-edited version must not
        // make the whole repository unusable; an unparseable file reads as an
        // empty virtual branches the user rebuilds from the UI.
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    pub(super) fn write_virtual_branches_impl(&self, state: &VirtualBranchesState) -> Result<()> {
        state.validate()?;
        let dir = self.virtual_branches_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "creating the virtual branches directory {}: {e}",
                dir.display()
            )))
        })?;
        let json = serde_json::to_vec_pretty(state).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "serializing virtual branches: {e}"
            )))
        })?;
        write_atomically(&self.virtual_branches_state_path(), &json)
    }

    pub(super) fn read_assignments_impl(&self) -> Result<AssignmentIndex> {
        let path = self.virtual_branches_assignments_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // Same reasoning as `read_virtual_branches_impl`: absent is ordinary, any
            // other failure is not, and reporting it as "no assignments" would
            // quietly discard a file the user cannot read yet can.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AssignmentIndex::default());
            }
            Err(error) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "reading {}: {error}",
                    path.display()
                ))));
            }
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    pub(super) fn write_assignments_impl(&self, index: &AssignmentIndex) -> Result<()> {
        let dir = self.virtual_branches_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "creating the virtual branches directory {}: {e}",
                dir.display()
            )))
        })?;
        let json = serde_json::to_vec_pretty(index)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("serializing assignments: {e}"))))?;
        write_atomically(&self.virtual_branches_assignments_path(), &json)
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
    /// and in a virtual branches that is the *normal* state, not a mistake: the whole
    /// point is uncommitted changes spread across several branches at once.
    /// Every stacked branch edit would fail, so the replay goes through the same
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
    /// Uses `merge-tree` + `commit-tree` rather than `rebase` so a virtual branches
    /// branch can be restacked while the user has unrelated edits in the
    /// working tree — which is the normal state in a virtual branches.
    fn restack_branch_onto(&self, name: &str, onto: &str) -> Result<CommitId> {
        let branch_id = self.require_revision(name)?;
        let onto_id = self.require_revision(onto)?;

        // Nothing to replay when the branch is already on the base.
        if branch_id == onto_id {
            return Ok(branch_id);
        }

        let tree = match self.merge_tree(&branch_id, &onto_id)? {
            MergeOutcome::Merged(tree) => tree,
            MergeOutcome::Conflicted { paths } => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "'{name}' cannot be moved onto '{onto}': they conflict in {}",
                    describe_paths(&paths)
                ))));
            }
        };
        let commit = self.commit_tree(&tree, &[&onto_id], &format!("Rebase {name} onto {onto}"))?;

        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref")
            .arg(format!("refs/heads/{name}"))
            .arg(commit.as_ref())
            .arg(branch_id.as_ref());
        run_git_simple(cmd, "git update-ref")?;
        Ok(commit)
    }

    // ── The working directory ────────────────────────────────────

    /// Refuse an action that would land on `gitcomet/virtual` because it acts on
    /// **HEAD** rather than on a branch the user picked.
    ///
    /// Hiding the branch from every branch list keeps it out of the pickers, but a
    /// plain commit, an amend and an ordinary push all apply to whatever HEAD is —
    /// and entering the virtual branches is exactly what puts HEAD there. Without this,
    /// the user's own commits land on GitComet's bookkeeping branch, where the next
    /// rebuild discards them, and the ordinary push publishes that branch to the
    /// user's remote under their name.
    ///
    /// `action` names what was refused, so the message says what was refused rather
    /// than something generic. The escape is the Virtual branches tab, which works on the
    /// branch each change belongs to.
    ///
    /// A failure to name HEAD is treated as "not the virtual-branches branch" rather
    /// than propagated: `current_branch_name` needs a commit to read, so in a
    /// repository that has none yet — where a first commit is the one thing the
    /// user is trying to do — it fails, and failing there would block the very
    /// commit the guard exists to protect. Whatever command asked is about to
    /// run against the same repository and will report the real problem.
    pub(super) fn refuse_virtual_branches_head_action(&self, action: &str) -> Result<()> {
        if let Ok(Some(branch)) = self.current_branch_name()
            && is_virtual_branches_branch(branch.trim())
        {
            let product = gitcomet_core::identity::current().display_name();
            return Err(Error::new(ErrorKind::Backend(format!(
                "'{branch}' is {product}'s own branch and is rebuilt from the applied \
                 branches, so {action} here would be undone by the next apply. Use \
                 the Virtual branches tab: it works on the branch each change belongs to, \
                 and pushes a named branch rather than HEAD."
            ))));
        }
        Ok(())
    }

    /// Whether the working directory is currently sitting on the virtual-branches branch.
    ///
    /// This is what decides whether a rebuild should also move the files: a
    /// virtual branches the user is not looking at has no business changing their
    /// working tree, and one they are looking at has to follow.
    pub(super) fn virtual_branches_is_checked_out(&self) -> bool {
        self.current_branch_impl()
            .map(|branch| branch == VIRTUAL_BRANCHES_BRANCH)
            .unwrap_or(false)
    }

    /// Put the working directory on the virtual-branches branch, building it first if it
    /// does not exist yet.
    ///
    /// The checkout is git's own, so its rule applies: local changes that the
    /// switch would overwrite are refused rather than carried, and nothing is
    /// discarded. That refusal is the whole safety story of this feature — the
    /// user is told which move failed and why, and gitcomet/virtual has not been
    /// created and left half-applied behind.
    pub(super) fn enter_virtual_branches_impl(&self) -> Result<CommitId> {
        if self.resolve_revision(VIRTUAL_BRANCHES_BRANCH)?.is_none() {
            let state = self.read_virtual_branches_impl()?;
            let applied = state.application_order()?;
            let names: Vec<&str> = applied.iter().map(|branch| branch.name.as_str()).collect();
            self.update_virtual_branches_branch_impl(&names)?;
        }
        let tip = self.require_revision(VIRTUAL_BRANCHES_BRANCH)?;

        let mut cmd = self.git_plumbing();
        cmd.arg("checkout")
            // Submodules are the user's business, not a side effect of which
            // virtual branches happen to be applied.
            .arg("--no-recurse-submodules")
            .arg(VIRTUAL_BRANCHES_BRANCH);
        let product = gitcomet_core::identity::current().display_name();
        run_git_simple(cmd, "git checkout").map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "could not switch the working directory to the virtual_branches: {error}\n\n\
                 {product} will not discard uncommitted changes. Commit or stash \
                 them, then try again."
            )))
        })?;
        Ok(tip)
    }

    /// Take the working directory back off the virtual-branches branch and onto `onto`.
    ///
    /// `onto` is the branch the user was on before they entered the virtual branches, so
    /// this is the inverse of [`Self::enter_virtual_branches_impl`] and carries the same
    /// refusal rule.
    pub(super) fn leave_virtual_branches_impl(&self, onto: &str) -> Result<()> {
        self.require_revision(onto)?;
        let mut cmd = self.git_plumbing();
        cmd.arg("checkout").arg("--no-recurse-submodules").arg(onto);
        let product = gitcomet_core::identity::current().display_name();
        run_git_simple(cmd, "git checkout").map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "could not switch back to '{onto}': {error}\n\n\
                 {product} will not discard uncommitted changes. Commit or stash \
                 them, then try again."
            )))
        })
    }

    /// Move the working directory from one virtual branches tree to another.
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
    /// *before* moving the virtual branches ref, not after, or the two would disagree.
    pub(super) fn sync_virtual_branches_workdir_impl(
        &self,
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
        let product = gitcomet_core::identity::current().display_name();
        run_git_simple(cmd, "git read-tree").map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "could not update the working directory: {error}\n\n\
                 {product} will not discard uncommitted changes. Commit or stash \
                 the files above, then try again."
            )))
        })
    }

    // ── The virtual-branches branch ─────────────────────────────────────

    pub(super) fn update_virtual_branches_branch_impl(&self, applied: &[&str]) -> Result<CommitId> {
        let state = self.read_virtual_branches_impl()?;
        let target_id = self.require_revision(&state.target)?;

        // Start from the target every time: the virtual-branches branch is a pure
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
            let tree = match self.merge_tree(&branch_id, &head)? {
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
            // Every merge gets its own commit so the virtual branches history shows
            // which branch contributed what, and so unapplying one later is a
            // replay rather than an edit of history that has been pushed.
            let message = format!("Apply {name}");
            head = self.commit_tree(&tree, &[&head], &message)?;
            merged.push(name);
            merged_any = true;
        }

        if !merged_any {
            // Nothing changed hands, so the virtual-branches branch should point at
            // the target directly rather than accumulate no-op merge commits.
            let existing = self.resolve_revision(VIRTUAL_BRANCHES_BRANCH)?;
            if existing.as_ref() == Some(&head) {
                return Ok(head);
            }
        }

        // The working directory follows only when the user is looking at the
        // virtual branches; a rebuild of a virtual branches nobody has entered must not touch
        // their files.
        //
        // Syncing *before* the ref moves is the whole ordering here. A refused
        // sync leaves the index and the working tree exactly as they were, and
        // with the ref still on the old tree the two cannot disagree.
        let previous = self.resolve_revision(VIRTUAL_BRANCHES_BRANCH)?;
        if let Some(previous) = previous
            .as_ref()
            .filter(|_| self.virtual_branches_is_checked_out())
        {
            self.sync_virtual_branches_workdir_impl(previous, &head)?;
        }

        self.update_virtual_branches_ref(&head)?;
        Ok(head)
    }

    /// Write the virtual-branches branch, refusing to move it backwards.
    ///
    /// The old value is passed as `update-ref`'s expected-old argument so a
    /// concurrent rebuild cannot be silently overwritten.
    fn update_virtual_branches_ref(&self, commit: &CommitId) -> Result<()> {
        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref");
        cmd.arg(format!("refs/heads/{VIRTUAL_BRANCHES_BRANCH}"));
        cmd.arg(commit.as_ref());
        // The empty string means "must not exist yet", which is what creating
        // the branch needs; a real old value makes the update conditional on
        // the branch not having moved underneath us. A hand-written all-zero id
        // would have been the wrong way to say the first case: its length is
        // 40 for SHA-1 and 64 for SHA-256, so it only works in the repository
        // format the author happened to be thinking of.
        cmd.arg(match self.resolve_revision(VIRTUAL_BRANCHES_BRANCH)? {
            Some(existing) => existing.to_string(),
            None => String::new(),
        });
        run_git_simple(cmd, "git update-ref")
    }

    pub(super) fn update_virtual_branches_target_impl(&self, target: &str) -> Result<()> {
        let mut state = self.read_virtual_branches_impl()?;
        state.set_target(target)?;
        self.write_virtual_branches_impl(&state)?;

        // Fail before anything is rebased if the new target does not exist.
        self.require_revision(&state.target)?;
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
        self.update_virtual_branches_branch_impl(&names).map(|_| ())
    }

    // ── Assigning and committing ─────────────────────────────────

    pub(super) fn commit_paths_to_virtual_branch_impl(
        &self,
        name: &str,
        message: &str,
        paths: &[&Path],
    ) -> Result<CommitOperationOutcome> {
        let state = self.read_virtual_branches_impl()?;
        let branch = state.get(name).ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "'{name}' is not a virtual-branches branch"
            )))
        })?;
        let base = branch.base_branch(&state.target).to_string();
        let base_id = self.require_revision(&base)?;
        let pre_head = self.resolve_revision(name)?;

        // The commit is built against the branch's own base rather than the
        // virtual branches, so it contains only the assigned files. A temporary index
        // keeps the user's real index and working tree untouched.
        let index_path = self.virtual_branches_dir().join("commit-index");
        // Not `.ok()`: if the directory cannot be made, every later `GIT_INDEX_FILE`
        // command fails with a git error that says nothing about the real
        // problem, and the user is left guessing at their repository.
        std::fs::create_dir_all(self.virtual_branches_dir()).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "creating the virtual branches directory {}: {e}",
                self.virtual_branches_dir().display()
            )))
        })?;

        let assignments = self.read_assignments_impl()?;
        let tree = self.commit_paths_tree(&index_path, &base_id, paths, name, &assignments)?;
        let commit = self.commit_tree(&tree, &[&base_id], message)?;

        let mut cmd = self.git_plumbing();
        cmd.arg("update-ref")
            .arg(format!("refs/heads/{name}"))
            .arg(commit.as_ref());
        if let Some(pre) = &pre_head {
            cmd.arg(pre.as_ref());
        }
        run_git_simple(cmd, "git update-ref")?;

        let _ = std::fs::remove_file(&index_path);

        // The branch moved, so the virtual-branches branch has to be rebuilt to pick
        // the new commit up.
        let applied = state.application_order()?;
        let names: Vec<&str> = applied.iter().map(|b| b.name.as_str()).collect();
        self.update_virtual_branches_branch_impl(&names)?;

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
        let working_bytes = std::fs::read(self.spec.workdir.join(path)).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "reading {} from the working tree: {e}",
                path.display()
            )))
        })?;
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
        let keep = |fingerprint: gitcomet_core::virtual_branches::HunkFingerprint| {
            mine.iter().any(|(assigned, assigned_branch)| {
                *assigned == fingerprint && *assigned_branch == branch
            })
        };
        let synthesized = synthesize_for_branch(base_text, working_text, &spans, &keep);

        let scratch = index_path.with_extension("blob");
        std::fs::write(&scratch, synthesized.as_bytes()).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "writing {}: {e}",
                scratch.display()
            )))
        })?;
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
        staged.arg("ls-files").arg("-s").arg("--").arg(path);
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
    /// not read or written. That is what lets the virtual branches be rebuilt while
    /// the user has unsaved edits in the files being merged.
    ///
    /// Requires Git 2.38 or newer. Older Git reports it as an unknown option,
    /// which is a command failure and is propagated as one rather than being
    /// dressed up as a conflict between two of the user's branches.
    fn merge_tree(&self, ours: &CommitId, theirs: &CommitId) -> Result<MergeOutcome> {
        let mut cmd = self.git_plumbing();
        cmd.arg("merge-tree")
            .arg("--write-tree")
            .arg("--merge-base")
            .arg(merge_base_of(self, ours, theirs))
            .arg(ours.as_ref())
            .arg(theirs.as_ref());

        match run_git_capture(cmd, "git merge-tree") {
            Ok(out) => Ok(MergeOutcome::Merged(CommitId(
                out.lines().next().unwrap_or_default().trim().into(),
            ))),
            Err(error) if merge_tree_conflicted(&error) => Ok(MergeOutcome::Conflicted {
                paths: conflicted_paths(&error),
            }),
            Err(error) => Err(error),
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
        Ok(out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Create a commit from `tree` with the given parents.
    fn commit_tree(
        &self,
        tree: &CommitId,
        parents: &[&CommitId],
        message: &str,
    ) -> Result<CommitId> {
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
        virtual_branches: &VirtualBranchesState,
    ) -> Option<String> {
        let branch: &VirtualBranch = virtual_branches.get(name)?;
        Some(branch.base_branch(&virtual_branches.target).to_string())
    }

    pub(super) fn push_virtual_branch_impl(&self, name: &str) -> Result<CommandOutput> {
        crate::util::validate_ref_like_arg(name, "branch name")?;
        let Some(remote) = self.preferred_remote_name()? else {
            return Err(Error::new(ErrorKind::Backend(
                "this repository has no remote to push to".to_string(),
            )));
        };

        // The named ref on both sides, not `HEAD`: in a virtual branches the user is
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
             already in the virtual branches, in {}\n\n\
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
    /// A conflict is not an error condition in the caller: the virtual branches is
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

/// Whether a failed `git merge-tree --write-tree` means "these two clash".
///
/// `1` is git's conflict code and nothing else produces it: a usage error —
/// `--write-tree` needs Git 2.38, and an older one answers with exit 129 — or a
/// repository git cannot read both exit elsewhere. Folding those into
/// "conflict" would tell the user two of their branches are in each other's
/// way when in fact nothing was ever compared, and the way out of the message
/// (unapply a branch) would be wrong advice.
fn merge_tree_conflicted(error: &Error) -> bool {
    let ErrorKind::Git(failure) = error.kind() else {
        return false;
    };
    failure.exit_code() == Some(1)
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
/// truncated virtual branches file that reads as an empty virtual branches.
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

/// Whether `name` is one of GitComet's own branches rather than a user's.
pub(crate) fn is_virtual_branches_branch(name: &str) -> bool {
    name.starts_with(VIRTUAL_BRANCHES_REF_PREFIX)
}

#[cfg(test)]
mod tests;
