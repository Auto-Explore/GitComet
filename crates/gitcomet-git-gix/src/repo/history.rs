use super::patch::write_pathspec_file;
use super::{GixRepo, oid_to_arc_str};
use crate::util::{
    bytes_to_text_preserving_utf8, command_output, describe_path_list, git_command_failed_error,
    push_literal_pathspec_file, run_git_capture, run_git_capture_bytes, run_git_raw_output,
    run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, short_commit_id};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{
    CommandOutput, InteractiveRebaseAction, InteractiveRebaseEntry,
    REVERT_ABORT_KEPT_HEAD_SENTINEL, REVERT_NOTHING_TO_REVERT_SENTINEL, REVERT_SKIP_COMMAND,
    ResetMode, Result, SequencerState,
};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::process::Command;

/// Returns the HEAD commit id, or `None` when HEAD is unborn / empty.
pub(super) fn gix_head_id_or_none(repo: &gix::Repository) -> Result<Option<gix::ObjectId>> {
    crate::refs::head_oid(repo)
}

/// Upper bound on the number of commits a single squash may cover; a runaway
/// walk past this means `oldest` is not a first-parent ancestor of `head`.
const MAX_SQUASH_CHAIN: usize = 10_000;
const PERSISTED_CHERRY_PICK_MAINLINE: &str = "gitcomet-cherry-pick-mainline";
const PERSISTED_CHERRY_PICK_MAINLINE_STAGING: &str = "gitcomet-cherry-pick-mainline.staging";

pub(super) fn peel_commit<'r>(repo: &'r gix::Repository, spec: &str) -> Result<gix::Commit<'r>> {
    crate::refs::resolve_required(repo, spec)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))).with_source(e)
        })?
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object {spec}: {e}"))))?
        .peel_to_commit()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel commit {spec}: {e}"))))
}

/// Walks first parents from `head` down to `oldest` (inclusive), requiring a
/// strictly linear chain: every commit in the range, including `oldest`, must
/// have exactly one parent. Returns the chain (youngest first, hex ids) and
/// `oldest`'s parent id.
fn first_parent_chain_to(
    repo: &gix::Repository,
    head: &CommitId,
    oldest: &CommitId,
) -> Result<(Vec<String>, String)> {
    let oldest_hex = oldest.as_ref().to_ascii_lowercase();
    let mut current = head.as_ref().to_ascii_lowercase();
    let mut chain = Vec::new();

    loop {
        let commit = peel_commit(repo, &current)?;
        let mut parents = commit.parent_ids();
        let (Some(parent), None) = (parents.next(), parents.next()) else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "squash: commit {current} does not have exactly one parent"
            ))));
        };
        let parent = parent.detach().to_string();
        chain.push(current.clone());

        if current == oldest_hex {
            return Ok((chain, parent));
        }
        if chain.len() >= MAX_SQUASH_CHAIN {
            return Err(Error::new(ErrorKind::Backend(format!(
                "squash: {oldest_hex} is not a first-parent ancestor of {}",
                head.as_ref()
            ))));
        }
        current = parent;
    }
}

/// The oldest squashed commit's author, formatted for `GIT_AUTHOR_*` env vars
/// (`GIT_AUTHOR_DATE` in git's raw `<unix> <±HHMM>` format).
fn commit_author_env(repo: &gix::Repository, spec: &str) -> Result<(String, String, String)> {
    let commit = peel_commit(repo, spec)?;
    let author = commit
        .author()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix author {spec}: {e}"))))?;
    let time = author
        .time()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix author time {spec}: {e}"))))?;
    let sign = if time.offset < 0 { '-' } else { '+' };
    let offset_abs = time.offset.unsigned_abs();
    let date = format!(
        "{} {}{:02}{:02}",
        time.seconds,
        sign,
        offset_abs / 3600,
        (offset_abs % 3600) / 60
    );
    Ok((author.name.to_string(), author.email.to_string(), date))
}

pub(super) fn append_command_output(acc: &mut CommandOutput, output: CommandOutput) {
    if !acc.stdout.is_empty() && !output.stdout.is_empty() {
        acc.stdout.push('\n');
    }
    acc.stdout.push_str(&output.stdout);
    if !acc.stderr.is_empty() && !output.stderr.is_empty() {
        acc.stderr.push('\n');
    }
    acc.stderr.push_str(&output.stderr);
    acc.exit_code = output.exit_code;
}

fn cherry_pick_error(message: &str) -> Error {
    Error::new(ErrorKind::Backend(format!("cherry-pick: {message}")))
}

fn staged_overlap_error(paths: &[&[u8]]) -> Error {
    let (has, them) = if paths.len() == 1 {
        ("has", "it")
    } else {
        ("have", "them")
    };
    cherry_pick_error(&format!(
        "{} already {has} staged changes; commit or unstage {them} first, or cherry-pick \
         without committing to merge the pick into {them}",
        describe_path_list(paths)
    ))
}

/// The paths of a `-z --name-only` listing.
fn nul_separated_paths(listing: &[u8]) -> BTreeSet<Vec<u8>> {
    listing
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

pub(super) fn append_raw_output(acc: &mut CommandOutput, output: &crate::util::GitOutput) {
    append_command_output(acc, command_output("", output));
}

/// The sequencer commands that auto-skip empty steps.
#[derive(Clone, Copy)]
enum SequenceKind {
    CherryPick,
    Revert,
}

impl SequenceKind {
    fn subcommand(self) -> &'static str {
        match self {
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }

    fn skip_label(self) -> &'static str {
        match self {
            Self::CherryPick => "git cherry-pick --skip",
            Self::Revert => REVERT_SKIP_COMMAND,
        }
    }

    fn root_ref(self) -> crate::refs::RootRef {
        match self {
            Self::CherryPick => crate::refs::RootRef::CherryPick,
            Self::Revert => crate::refs::RootRef::Revert,
        }
    }
}

/// On-disk position of an in-progress cherry-pick or revert, compared before
/// and after a continue to tell "advanced and paused at a later step" from
/// "failed in place".
#[derive(PartialEq)]
struct SequencerProgress {
    /// Steps left in `sequencer/todo`; `None` for a single pick or revert,
    /// which keeps no todo.
    remaining_steps: Option<usize>,
    /// `CHERRY_PICK_HEAD`/`REVERT_HEAD` — the commit it is stopped on.
    stopped_on: Option<String>,
}

impl SequencerProgress {
    fn advanced_from(&self, before: &SequencerProgress) -> bool {
        match (before.remaining_steps, self.remaining_steps) {
            (Some(before_remaining), Some(remaining)) if remaining < before_remaining => {
                return true;
            }
            _ => {}
        }
        self.stopped_on != before.stopped_on
    }
}

const CHERRY_PICK_ALREADY_APPLIED_SENTINEL: &str = "GITCOMET_CHERRY_PICK_ALREADY_APPLIED";

impl GixRepo {
    pub(super) fn head_commit_id_impl(&self) -> Result<Option<CommitId>> {
        let repo = self.reopen_repo()?;
        gix_head_id_or_none(&repo).map(|id| id.map(|id| CommitId(oid_to_arc_str(&id))))
    }

    pub(super) fn commit_is_ancestor(
        &self,
        ancestor: &CommitId,
        descendant: &CommitId,
    ) -> Result<bool> {
        validate_hex_commit_id(ancestor)?;
        validate_hex_commit_id(descendant)?;

        let label = format!("git merge-base --is-ancestor {ancestor} {descendant}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge-base")
            .arg("--is-ancestor")
            .arg(ancestor.as_ref())
            .arg(descendant.as_ref());
        let output = run_git_raw_output(cmd, &label)?;
        if output.status.success() {
            return Ok(true);
        }
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        Err(git_command_failed_error(&label, output))
    }

    pub(super) fn reset_with_output_impl(
        &self,
        target: &str,
        mode: ResetMode,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(target, "reset target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("reset");
        let mode_flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        cmd.arg(mode_flag).arg(target);
        let label = format!("git reset {mode_flag} {target}");
        run_git_with_output(cmd, &label)
    }

    pub(super) fn squash_message_preview_impl(
        &self,
        oldest: &CommitId,
        head: &CommitId,
    ) -> Result<String> {
        validate_hex_commit_id(oldest)?;
        validate_hex_commit_id(head)?;

        let repo = self.repo();
        let (chain, _oldest_parent) = first_parent_chain_to(&repo, head, oldest)?;
        let mut messages = Vec::with_capacity(chain.len());
        for spec in &chain {
            let commit = peel_commit(&repo, spec)?;
            messages.push(
                bytes_to_text_preserving_utf8(commit.message_raw_sloppy().as_ref())
                    .trim_end()
                    .to_string(),
            );
        }
        messages.reverse();
        Ok(gitcomet_core::squash::build_squash_message(&messages))
    }

    pub(super) fn squash_commits_with_output_impl(
        &self,
        oldest: &CommitId,
        expected_head: &CommitId,
        message: &str,
    ) -> Result<CommandOutput> {
        validate_hex_commit_id(oldest)?;
        validate_hex_commit_id(expected_head)?;
        if message.trim().is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "squash: commit message must not be empty".to_string(),
            )));
        }

        // Re-validate against live repo state: the selection was made from a
        // possibly stale log snapshot.
        let repo = self.reopen_repo()?;
        let head = gix_head_id_or_none(&repo)?
            .ok_or_else(|| Error::new(ErrorKind::Backend("squash: HEAD is unborn".to_string())))?;
        if !head
            .to_string()
            .eq_ignore_ascii_case(expected_head.as_ref())
        {
            return Err(Error::new(ErrorKind::Backend(
                "squash aborted: HEAD moved since the squash was prepared".to_string(),
            )));
        }
        let (chain, oldest_parent) = first_parent_chain_to(&repo, expected_head, oldest)?;
        let count = chain.len();
        if count < 2 {
            return Err(Error::new(ErrorKind::Backend(
                "squash: needs at least two commits".to_string(),
            )));
        }

        let (author_name, author_email, author_date) = commit_author_env(&repo, oldest.as_ref())?;

        // Respect commit.gpgsign: `git commit-tree` never signs unless asked,
        // so without this a signed-commit repo would get an unsigned squash
        // commit and later have the push rejected.
        let sign = repo
            .config_snapshot()
            .boolean("commit.gpgsign")
            .unwrap_or(false);

        // The squash commit reuses HEAD's tree with the range's base as its
        // parent, so the worktree and index are never touched.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit-tree");
        if sign {
            cmd.arg("-S");
        }
        cmd.arg(format!("{}^{{tree}}", expected_head.as_ref()))
            .arg("-p")
            .arg(&oldest_parent)
            .arg("-m")
            .arg(message)
            .env("GIT_AUTHOR_NAME", author_name)
            .env("GIT_AUTHOR_EMAIL", author_email)
            .env("GIT_AUTHOR_DATE", author_date);
        let new_sha = run_git_capture(cmd, "git commit-tree")?.trim().to_string();
        if new_sha.is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "squash: git commit-tree produced no commit id".to_string(),
            )));
        }

        self.require_bare_head_available()?;

        // Atomic compare-and-swap on HEAD (dereferences to the branch ref when
        // attached): fails without side effects if HEAD moved concurrently.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("update-ref")
            .arg("-m")
            .arg(format!("squash: {count} commits"))
            .arg("HEAD")
            .arg(&new_sha)
            .arg(expected_head.as_ref());
        run_git_with_output(cmd, &format!("git update-ref HEAD {new_sha}"))
    }

    pub(super) fn rebase_with_output_impl(&self, onto: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(onto, "rebase target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--").arg(onto);
        run_git_with_output(cmd, &format!("git rebase {onto}"))
    }

    pub(super) fn cherry_pick_with_output_impl(
        &self,
        id: &CommitId,
        commit: bool,
        mainline: Option<usize>,
    ) -> Result<CommandOutput> {
        validate_hex_commit_id(id)?;
        let parent_ids = self.validate_single_pick_mainline("cherry-pick", id, mainline)?;
        let mainline_label = mainline.map_or_else(String::new, |parent| format!(" -m {parent}"));
        let label = if commit {
            format!("git cherry-pick{mainline_label} {}", id.as_ref())
        } else {
            format!(
                "git cherry-pick{mainline_label} --no-commit {}",
                id.as_ref()
            )
        };

        // The signing-passphrase retry replays this call after git stopped
        // at the commit step; finish that pick rather than refuse it.
        if commit && self.cherry_pick_awaits_commit(id)? {
            let mut cmd = self.git_workdir_cmd();
            cmd.env("GIT_EDITOR", "true");
            cmd.arg("cherry-pick").arg("--continue");
            return self.run_sequence_step_output(
                SequenceKind::CherryPick,
                cmd,
                "git cherry-pick --continue",
            );
        }

        if let Some(operation) = self.operation_in_progress_label()? {
            return Err(Error::new(ErrorKind::Backend(format!(
                "cherry-pick: {operation} is in progress; finish or abort it first"
            ))));
        }
        // Git refuses to commit a pick over staged work. `--no-commit` needs
        // no such path: it merges into the staged work like `cherry-pick -n`.
        if commit && !self.index_matches_head()? {
            return self.cherry_pick_beside_staged_work(id, mainline, &parent_ids, &label);
        }

        // A single merge pick has no sequencer todo from which continue-time
        // code can recover `-m`. Keep the exact source/parent pair beside
        // Git's state so an empty resolution can still be classified against
        // the selected mainline. Never overwrite metadata belonging to an
        // operation that was already in progress; the command below will
        // report that collision itself.
        let started_with_sequencer = self.rebase_in_progress_impl()?;
        if !started_with_sequencer {
            self.clear_persisted_cherry_pick_mainline();
            if let Some(parent) = mainline.and_then(|number| parent_ids.get(number - 1)) {
                self.persist_cherry_pick_mainline(id.as_ref(), parent)?;
            }
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cherry-pick");
        if let Some(parent) = mainline {
            cmd.arg("-m").arg(parent.to_string());
        }
        if commit {
            // A source commit that was created empty on purpose stops the
            // pick with the same "now empty" status as an already-applied
            // one; `--allow-empty` commits it instead, so only picks that
            // *become* empty reach the already-applied handling below.
            cmd.arg("--allow-empty");
        } else {
            cmd.arg("--no-commit");
        }
        cmd.arg("--").arg(id.as_ref());

        let output = run_git_raw_output(cmd, &label).map_err(|e| {
            Error::new(ErrorKind::Backend(format!("failed to run {label}: {e}"))).with_source(e)
        })?;
        if output.status.success() {
            if !started_with_sequencer {
                self.clear_persisted_cherry_pick_mainline();
            }
            return Ok(command_output(&label, &output));
        }

        if self.sequence_stopped_became_empty(SequenceKind::CherryPick)? {
            if self.rebase_in_progress_impl()? {
                let mut abort = self.git_workdir_cmd();
                abort.arg("cherry-pick").arg("--abort");
                run_git_with_output(abort, "git cherry-pick --abort")?;
            }
            if !started_with_sequencer {
                self.clear_persisted_cherry_pick_mainline();
            }
            return Ok(CommandOutput {
                command: label,
                stdout: CHERRY_PICK_ALREADY_APPLIED_SENTINEL.to_string(),
                stderr: bytes_to_text_preserving_utf8(&output.stderr),
                exit_code: Some(0),
            });
        }

        if !started_with_sequencer && !self.cherry_pick_in_progress_impl()? {
            self.clear_persisted_cherry_pick_mainline();
        }
        Err(git_command_failed_error(&label, output))
    }

    /// Commits only `id`'s change while unrelated work stays staged: a
    /// `--no-commit` pick, then `commit --only` of the paths it changed.
    /// A pick that conflicts or reaches a staged path is rolled back, so the
    /// staged work never ends up in the commit.
    fn cherry_pick_beside_staged_work(
        &self,
        id: &CommitId,
        mainline: Option<usize>,
        parent_ids: &[String],
        label: &str,
    ) -> Result<CommandOutput> {
        if gix_head_id_or_none(&self.repo())?.is_none() {
            return Err(cherry_pick_error(
                "the index has staged changes; commit or unstage them first",
            ));
        }
        if self.index_has_conflicts() {
            return Err(cherry_pick_error(
                "the index has unresolved conflicts; resolve them first",
            ));
        }
        let staged = self.index_paths_changed_from("HEAD")?;
        let source_parent = match mainline {
            Some(number) => parent_ids.get(number - 1),
            None => parent_ids.first(),
        };
        let touched = self.commit_changed_paths(source_parent.map(String::as_str), id.as_ref())?;
        let overlap: Vec<&[u8]> = touched.intersection(&staged).map(Vec::as_slice).collect();
        if !overlap.is_empty() {
            return Err(staged_overlap_error(&overlap));
        }

        let mut write_tree = self.git_workdir_cmd();
        write_tree.arg("write-tree");
        let base_tree = run_git_capture(write_tree, "git write-tree")?
            .trim()
            .to_string();
        // `-n` rewrites MERGE_MSG, which may hold an earlier uncommitted
        // pick's message for the commit box.
        let merge_msg_path = self.repo().path().join("MERGE_MSG");
        let saved_merge_msg = fs::read(&merge_msg_path).ok();
        let restore_merge_msg = || {
            let _ = match &saved_merge_msg {
                Some(bytes) => fs::write(&merge_msg_path, bytes),
                None => fs::remove_file(&merge_msg_path),
            };
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cherry-pick");
        if let Some(parent) = mainline {
            cmd.arg("-m").arg(parent.to_string());
        }
        cmd.arg("--no-commit").arg("--").arg(id.as_ref());
        let picked = run_git_raw_output(cmd, label)?;
        let changed = self.index_paths_changed_from(&base_tree)?;
        if !picked.status.success() {
            let conflicted = self.index_has_conflicts();
            if !changed.is_empty() {
                self.restore_paths_from_tree(&base_tree, &changed)?;
            }
            restore_merge_msg();
            if conflicted {
                return Err(cherry_pick_error(&format!(
                    "{} conflicts with the current branch. Your staged changes were left as \
                     they were; commit or unstage them and cherry-pick again to resolve the \
                     conflicts, or cherry-pick without committing",
                    id.short()
                )));
            }
            return Err(git_command_failed_error(label, picked));
        }
        // Rename detection can carry the change onto a staged path.
        let folded: Vec<&[u8]> = changed.intersection(&staged).map(Vec::as_slice).collect();
        if !folded.is_empty() {
            let error = staged_overlap_error(&folded);
            self.restore_paths_from_tree(&base_tree, &changed)?;
            restore_merge_msg();
            return Err(error);
        }

        let mut output = command_output(label, &picked);
        // An empty source is committed on purpose, as `--allow-empty` does on
        // the clean-index path; any other empty result was already applied.
        if changed.is_empty() && !touched.is_empty() {
            restore_merge_msg();
            output.stdout = CHERRY_PICK_ALREADY_APPLIED_SENTINEL.to_string();
            return Ok(output);
        }
        let committed = self.commit_picked_paths(id, &changed);
        if committed.is_err() {
            self.restore_paths_from_tree(&base_tree, &changed)?;
        }
        restore_merge_msg();
        append_command_output(&mut output, committed?);
        Ok(output)
    }

    /// `git commit --only` of `paths` with `id`'s message and authorship;
    /// no paths makes an empty commit.
    fn commit_picked_paths(
        &self,
        id: &CommitId,
        paths: &BTreeSet<Vec<u8>>,
    ) -> Result<CommandOutput> {
        let pathspec = if paths.is_empty() {
            None
        } else {
            Some(write_pathspec_file(paths.iter().map(Vec::as_slice))?)
        };
        let mut cmd = self.git_workdir_cmd();
        cmd.env("GIT_REFLOG_ACTION", "cherry-pick");
        cmd.args(["commit", "--no-verify", "--only", "--allow-empty", "-C"])
            .arg(id.as_ref());
        if let Some(pathspec) = &pathspec {
            push_literal_pathspec_file(&mut cmd, pathspec.path());
        }
        run_git_with_output(cmd, &format!("git commit --only -C {}", id.as_ref()))
    }

    /// Paths whose index entry differs from `tree_ish`, conflicts included.
    fn index_paths_changed_from(&self, tree_ish: &str) -> Result<BTreeSet<Vec<u8>>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args([
            "diff-index",
            "--cached",
            "-z",
            "--name-only",
            "--no-renames",
            "--ignore-submodules=none",
        ])
        .arg(tree_ish)
        .arg("--");
        let listing = run_git_capture_bytes(cmd, "git diff-index --cached --name-only")?;
        Ok(nul_separated_paths(&listing))
    }

    /// Paths `id` changes against `parent`, or against nothing for a root.
    fn commit_changed_paths(&self, parent: Option<&str>, id: &str) -> Result<BTreeSet<Vec<u8>>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args([
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            "--no-commit-id",
        ]);
        match parent {
            Some(parent) => cmd.arg(parent),
            None => cmd.arg("--root"),
        };
        cmd.arg(id);
        let listing = run_git_capture_bytes(cmd, "git diff-tree --name-only")?;
        Ok(nul_separated_paths(&listing))
    }

    /// Puts `paths` back to `tree` in the index and worktree. Callers pass
    /// only paths git merged into, which matched `tree` in both before.
    fn restore_paths_from_tree(&self, tree: &str, paths: &BTreeSet<Vec<u8>>) -> Result<()> {
        let pathspec = write_pathspec_file(paths.iter().map(Vec::as_slice))?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("restore")
            .arg(format!("--source={tree}"))
            .args(["--staged", "--worktree"]);
        push_literal_pathspec_file(&mut cmd, pathspec.path());
        run_git_with_output(cmd, "git restore --staged --worktree").map(|_| ())
    }

    /// Whether `id` is a single pick stopped only at its commit step.
    fn cherry_pick_awaits_commit(&self, id: &CommitId) -> Result<bool> {
        let repo = self.repo();
        let git_dir = repo.path();
        let Some(stopped_on) = crate::refs::root_ref(&repo, crate::refs::RootRef::CherryPick)?
        else {
            return Ok(false);
        };
        let same_commit = match (
            peel_commit(&repo, &stopped_on.to_string()),
            peel_commit(&repo, id.as_ref()),
        ) {
            (Ok(stopped), Ok(requested)) => stopped.id == requested.id,
            _ => false,
        };
        Ok(same_commit && !git_dir.join("sequencer").exists() && !self.index_has_conflicts())
    }

    /// Validates Git's 1-based `-m` parent for a single pick or revert of `id`
    /// before invoking git, so a stale or malformed UI request cannot leave
    /// sequencer state behind. Returns the commit's parent ids.
    fn validate_single_pick_mainline(
        &self,
        op: &str,
        id: &CommitId,
        mainline: Option<usize>,
    ) -> Result<Vec<String>> {
        let repo = self.repo();
        let parent_ids = peel_commit(&repo, id.as_ref())?
            .parent_ids()
            .map(|parent| parent.detach().to_string())
            .collect::<Vec<_>>();
        let parent_count = parent_ids.len();
        let short = id.as_ref().get(..8).unwrap_or(id.as_ref());
        match (parent_count > 1, mainline) {
            (true, None) => Err(Error::new(ErrorKind::Backend(format!(
                "{op}: {short} is a merge commit with {parent_count} parents; choose a mainline \
                 parent"
            )))),
            (true, Some(parent)) if parent == 0 || parent > parent_count => {
                Err(Error::new(ErrorKind::Backend(format!(
                    "{op}: mainline parent {parent} is invalid for merge commit {short}; choose a \
                     parent from 1 to {parent_count}"
                ))))
            }
            (false, Some(_)) => Err(Error::new(ErrorKind::Backend(format!(
                "{op}: {short} is not a merge commit; a mainline parent cannot be selected"
            )))),
            _ => Ok(parent_ids),
        }
    }

    /// `revert --no-commit`, then `commit --no-verify -F MERGE_MSG`: REVERT_HEAD
    /// appears only once git's merge ran, so refusals leave no state, and a
    /// failed commit step (hook, signer) resumes when this call is replayed.
    /// Without `commit` the inverse is just staged, like `cherry-pick -n`.
    pub(super) fn revert_with_output_impl(
        &self,
        id: &CommitId,
        commit: bool,
        mainline: Option<usize>,
    ) -> Result<CommandOutput> {
        validate_hex_commit_id(id)?;
        self.validate_single_pick_mainline("revert", id, mainline)?;
        let mainline_label = mainline.map_or_else(String::new, |parent| format!(" -m {parent}"));
        let label = if commit {
            format!("git revert{mainline_label} {}", id.as_ref())
        } else {
            format!("git revert{mainline_label} --no-commit {}", id.as_ref())
        };

        // The signing-passphrase retry replays this call after its commit
        // step failed; pick up at that step with the same flags.
        if commit && self.revert_awaits_commit(id)? {
            let mut output = self.commit_paused_revert()?;
            output.command = label;
            return Ok(output);
        }

        // `--no-commit` checks neither of these itself: it would fold staged
        // work into the revert and ignores another operation's state (a
        // closing `--quit` would even delete a leftover sequence).
        if let Some(operation) = self.operation_in_progress_label()? {
            return Err(Error::new(ErrorKind::Backend(format!(
                "revert: {operation} is in progress; finish or abort it first"
            ))));
        }
        if !self.index_matches_head()? {
            return Err(Error::new(ErrorKind::Backend(
                "revert: the index has staged changes; commit or unstage them first".to_string(),
            )));
        }

        let foreign_sequencer_dir = self.repo().path().join("sequencer").exists();
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("revert").arg("--no-commit");
        if let Some(parent) = mainline {
            cmd.arg("-m").arg(parent.to_string());
        }
        cmd.arg("--").arg(id.as_ref());
        let mut output = run_git_with_output(cmd, &label)?;
        if self.index_matches_head()? {
            // `--quit` also clears a sequencer directory this revert did not
            // create, so drop only what a single revert wrote.
            let mut clear = self.git_workdir_cmd();
            if foreign_sequencer_dir {
                clear.args(["update-ref", "-d", "REVERT_HEAD"]);
                run_git_with_output(clear, "git update-ref -d REVERT_HEAD")?;
            } else {
                clear.arg("revert").arg("--quit");
                run_git_with_output(clear, "git revert --quit")?;
            }
            output.stdout = REVERT_NOTHING_TO_REVERT_SENTINEL.to_string();
            return Ok(output);
        }
        if !commit {
            // MERGE_MSG stays as the next commit's template, as after `cherry-pick -n`.
            let mut drop_head = self.git_workdir_cmd();
            drop_head.args(["update-ref", "-d", "REVERT_HEAD"]);
            run_git_with_output(drop_head, "git update-ref -d REVERT_HEAD")?;
            return Ok(output);
        }
        append_command_output(&mut output, self.commit_paused_revert()?);
        Ok(output)
    }

    /// Commits a paused revert with the message, hooks, and `revert:` reflog
    /// entry of a one-shot `git revert` (`--no-edit` would also prepend a
    /// stale SQUASH_MSG).
    fn commit_paused_revert(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.env("GIT_REFLOG_ACTION", "revert");
        cmd.args(["commit", "--no-verify", "-F"])
            .arg(self.repo().path().join("MERGE_MSG"));
        run_git_with_output(cmd, "git commit --no-verify -F MERGE_MSG")
    }

    /// Whether `id` is a single revert stopped only at its commit step. The
    /// ids are peeled, so an abbreviated one still names the stopped revert.
    fn revert_awaits_commit(&self, id: &CommitId) -> Result<bool> {
        let repo = self.repo();
        let git_dir = repo.path();
        let Some(stopped_on) = crate::refs::root_ref(&repo, crate::refs::RootRef::Revert)? else {
            return Ok(false);
        };
        let same_commit = match (
            peel_commit(&repo, &stopped_on.to_string()),
            peel_commit(&repo, id.as_ref()),
        ) {
            (Ok(stopped), Ok(requested)) => stopped.id == requested.id,
            _ => false,
        };
        Ok(same_commit
            && !git_dir.join("sequencer").exists()
            && !self.index_has_conflicts()
            && !self.index_matches_head()?)
    }

    /// A single revert has no todo to advance past an empty resolution:
    /// `revert --continue` refuses it ("nothing to commit") and the UI offers
    /// only Continue and Abort, so such a stop is skipped instead. A sequence
    /// continues through [`Self::run_sequence_step_output`], which skips empty
    /// steps and reports a pause at the next conflict as progress.
    fn revert_continue_with_output(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let single = !self.repo().path().join("sequencer").exists();
        if single
            && self.revert_head_exists()?
            && !self.index_has_conflicts()
            && self.index_matches_head()?
        {
            cmd.arg("revert").arg("--skip");
            return run_git_with_output(cmd, REVERT_SKIP_COMMAND);
        }
        // Git already skips the editor without a tty; this covers platforms
        // where that check misfires.
        cmd.env("GIT_EDITOR", "true");
        cmd.arg("revert").arg("--continue");
        self.run_sequence_step_output(SequenceKind::Revert, cmd, "git revert --continue")
    }

    /// The in-progress operation a new pick or revert would collide with.
    /// Matches what git itself reports: a sequencer directory whose todo git
    /// cannot read is not an operation, even though it blocks new sequences.
    pub(super) fn operation_in_progress_label(&self) -> Result<Option<&'static str>> {
        use gix::state::InProgress;
        Ok(match crate::refs::operation_state(&self.repo())? {
            Some(InProgress::Rebase | InProgress::RebaseInteractive) => Some("a rebase"),
            Some(InProgress::ApplyMailbox | InProgress::ApplyMailboxRebase) => {
                Some("a patch apply")
            }
            Some(InProgress::CherryPick | InProgress::CherryPickSequence) => Some("a cherry-pick"),
            Some(InProgress::Merge) => Some("a merge"),
            Some(InProgress::Revert | InProgress::RevertSequence) => Some("a revert"),
            // gix reports a bisect ahead of REVERT_HEAD.
            Some(InProgress::Bisect) if self.revert_head_exists()? => Some("a revert"),
            Some(InProgress::Bisect) | None => match self.leftover_sequence_state() {
                SequencerState::CherryPick => Some("a cherry-pick sequence"),
                SequencerState::Revert => Some("a revert sequence"),
                _ => None,
            },
        })
    }

    /// The commit of the todo's current step. A sequence that stops because a
    /// step is already undone writes no `REVERT_HEAD`, so this names it.
    fn pending_revert_todo_commit(&self) -> Option<String> {
        let todo = fs::read_to_string(self.repo().path().join("sequencer").join("todo")).ok()?;
        let line = todo
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))?;
        let mut parts = line.split_whitespace();
        let command = parts.next()?;
        (command == "revert")
            .then(|| parts.next())?
            .map(str::to_string)
    }

    /// gix reports a bisect ahead of `REVERT_HEAD`, so probe the file itself.
    fn revert_head_exists(&self) -> Result<bool> {
        Ok(crate::refs::root_ref(&self.repo(), crate::refs::RootRef::Revert)?.is_some())
    }

    /// A cherry-pick or revert sequence outlives its `*_HEAD` when a stopped
    /// step is concluded with a plain `git commit`; git still reports it from
    /// the todo's first command, and so does this.
    fn leftover_sequence_state(&self) -> SequencerState {
        let todo = fs::read_to_string(self.repo().path().join("sequencer").join("todo"))
            .unwrap_or_default();
        match todo.split_whitespace().next() {
            Some("pick" | "p") => SequencerState::CherryPick,
            Some("revert") => SequencerState::Revert,
            _ => SequencerState::None,
        }
    }

    /// Whether any operation state is on disk that a bare `git reset` (which
    /// clears MERGE_HEAD, CHERRY_PICK_HEAD, REVERT_HEAD and the sequencer)
    /// would silently end.
    pub(super) fn operation_state_on_disk(&self) -> Result<bool> {
        let repo = self.repo();
        Ok(!matches!(
            crate::refs::operation_state(&repo)?,
            None | Some(gix::state::InProgress::Bisect)
        ) || self.revert_head_exists()?
            || repo.path().join("sequencer").exists())
    }

    /// Whether the index records exactly HEAD's tree, gitlinks included.
    pub(super) fn index_matches_head(&self) -> Result<bool> {
        self.index_matches(None)
    }

    /// Whether the index records exactly `commit`'s tree, or HEAD's when
    /// `None`, gitlinks included.
    pub(super) fn index_matches(&self, commit: Option<&str>) -> Result<bool> {
        let label = "git diff --cached --quiet --ignore-submodules=none";
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["diff", "--cached", "--quiet", "--ignore-submodules=none"]);
        cmd.args(commit);
        let output = run_git_raw_output(cmd, label)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_command_failed_error(label, output)),
        }
    }

    pub(super) fn rebase_continue_with_output_impl(&self) -> Result<CommandOutput> {
        // `cherry-pick --continue` refuses a revert sequence, and the fallback
        // chain below would report "no rebase in progress" over its error.
        if self.sequencer_state_impl()? == SequencerState::Revert {
            return self.revert_continue_with_output();
        }
        // A plain `git am` is neither a rebase nor a cherry-pick, so the chain
        // below could never continue it.
        if crate::refs::operation_state(&self.repo())? == Some(gix::state::InProgress::ApplyMailbox)
        {
            let mut cmd = self.git_workdir_cmd();
            cmd.env("GIT_EDITOR", "true");
            cmd.arg("am").arg("--continue");
            return run_git_with_output(cmd, "git am --continue");
        }
        let cmd = self.rebase_continue_command()?;
        match self.run_rebase_step_output(cmd, "git rebase --continue") {
            Ok(output) => Ok(output),
            Err(rebase_error) => {
                let mut cherry_pick_cmd = self.git_workdir_cmd();
                cherry_pick_cmd.arg("cherry-pick").arg("--continue");
                match self.run_sequence_step_output(
                    SequenceKind::CherryPick,
                    cherry_pick_cmd,
                    "git cherry-pick --continue",
                ) {
                    Ok(output) => Ok(output),
                    // A cherry-pick genuinely in progress owns this continue:
                    // its failure (unresolved files, a failed hook) is the
                    // actionable one, not "no rebase in progress".
                    Err(cherry_pick_error) if self.cherry_pick_in_progress_impl()? => {
                        Err(cherry_pick_error)
                    }
                    Err(_) => Err(rebase_error),
                }
            }
        }
    }

    /// Runs a cherry-pick or revert step, skipping empty stops
    /// ([`Self::run_sequence_auto_skip`]). Like the rebase path, a non-zero exit
    /// is success only when the sequence advanced and paused at a later
    /// conflict; no progress (unresolved conflicts, a failed hook re-running
    /// the same step) keeps git's error.
    fn run_sequence_step_output(
        &self,
        kind: SequenceKind,
        cmd: Command,
        label: &str,
    ) -> Result<CommandOutput> {
        let marker_before = self.sequencer_progress_marker(kind)?;
        let (output, last) = self.run_sequence_auto_skip(kind, cmd, label)?;
        let cherry_pick_in_progress = match kind {
            SequenceKind::CherryPick => {
                let in_progress = self.cherry_pick_in_progress_impl()?;
                if !in_progress {
                    self.clear_persisted_cherry_pick_mainline();
                }
                Some(in_progress)
            }
            SequenceKind::Revert => None,
        };
        if last.status.success() {
            return Ok(output);
        }
        let still_in_progress = match cherry_pick_in_progress {
            Some(in_progress) => in_progress,
            None => self.sequencer_state_impl()? == SequencerState::Revert,
        };
        let paused_after_progress = still_in_progress
            && match (marker_before, self.sequencer_progress_marker(kind)?) {
                // The command started the sequence and paused at a conflict.
                (None, Some(_)) => self.index_has_conflicts(),
                // Native todos hold only pick or revert steps, so advancing and
                // then stopping without an unmerged index is a later failure
                // (a hook or signer) whose error must be kept.
                (Some(before), Some(after)) => {
                    after.advanced_from(&before) && self.index_has_conflicts()
                }
                (_, None) => false,
            };
        if paused_after_progress {
            Ok(output)
        } else {
            Err(git_command_failed_error(label, last))
        }
    }

    /// Runs `cmd` and skips every step that stops empty, its changes already
    /// applied or undone: `--continue` refuses such a step and the UI has no
    /// skip control. Returns every command's output and the last one's.
    fn run_sequence_auto_skip(
        &self,
        kind: SequenceKind,
        cmd: Command,
        label: &str,
    ) -> Result<(CommandOutput, crate::util::GitOutput)> {
        let mut acc = CommandOutput {
            command: label.to_string(),
            ..CommandOutput::default()
        };
        let mut last = run_git_raw_output(cmd, label)?;
        append_raw_output(&mut acc, &last);
        while !last.status.success() && self.sequence_stopped_became_empty(kind)? {
            let marker = self.sequencer_progress_marker(kind)?;
            let mut skip = self.git_workdir_cmd();
            skip.arg(kind.subcommand()).arg("--skip");
            last = run_git_raw_output(skip, kind.skip_label())?;
            append_raw_output(&mut acc, &last);
            // A skip that moved nothing forward would loop forever.
            if self.sequencer_progress_marker(kind)? == marker {
                break;
            }
        }
        acc.exit_code = last.status.code();
        Ok((acc, last))
    }

    /// Whether the stopped step's source had changes but applying it leaves
    /// the index at HEAD. Checking cleanliness alone would also skip an
    /// intentionally empty source whose commit failed in a hook or signer.
    fn sequence_stopped_became_empty(&self, kind: SequenceKind) -> Result<bool> {
        let in_progress = match kind {
            SequenceKind::CherryPick => self.cherry_pick_in_progress_impl()?,
            SequenceKind::Revert => true,
        };
        if !in_progress || self.index_has_conflicts() {
            return Ok(false);
        }
        let stopped_on = self
            .sequencer_progress_marker(kind)?
            .and_then(|progress| progress.stopped_on);
        let stopped_on = match kind {
            SequenceKind::CherryPick => stopped_on,
            SequenceKind::Revert => stopped_on.or_else(|| self.pending_revert_todo_commit()),
        };
        let Some(stopped_on) = stopped_on else {
            return Ok(false);
        };

        // Without an explicit parent `diff-tree` emits no merge diff and calls
        // every merge empty. Only a merge pick GitComet started persists its
        // parent; guessing one could hide a real hook or signing failure.
        let repo = self.repo();
        let parents = peel_commit(&repo, &stopped_on)?
            .parent_ids()
            .map(|parent| parent.detach().to_string())
            .collect::<Vec<_>>();
        let source_parent = match (parents.as_slice(), kind) {
            ([], _) => None,
            ([parent], _) => Some(parent.clone()),
            (_, SequenceKind::CherryPick) => {
                match self.persisted_cherry_pick_mainline_parent(&stopped_on) {
                    Some(parent) if parents.contains(&parent) => Some(parent),
                    _ => return Ok(false),
                }
            }
            (_, SequenceKind::Revert) => return Ok(false),
        };
        let source_label = source_parent.as_ref().map_or_else(
            || format!("git diff-tree --quiet --root {stopped_on}"),
            |parent| format!("git diff-tree --quiet {parent} {stopped_on}"),
        );
        let mut source_diff = self.git_workdir_cmd();
        source_diff.args(["diff-tree", "--quiet"]);
        if let Some(parent) = source_parent {
            source_diff.arg(parent);
        } else {
            source_diff.arg("--root");
        }
        source_diff.arg(&stopped_on);
        let source_output = run_git_raw_output(source_diff, &source_label)?;
        match source_output.status.code() {
            Some(0) => return Ok(false),
            Some(1) => {}
            _ => return Err(git_command_failed_error(&source_label, source_output)),
        }
        self.index_matches_head()
    }

    fn persist_cherry_pick_mainline(&self, source: &str, parent: &str) -> Result<()> {
        let repo = self.repo();
        let path = repo.path().join(PERSISTED_CHERRY_PICK_MAINLINE);
        let staging = repo.path().join(PERSISTED_CHERRY_PICK_MAINLINE_STAGING);
        fs::write(&staging, format!("{source}\n{parent}\n"))
            .and_then(|()| {
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                fs::rename(&staging, &path)
            })
            .map_err(|error| {
                Error::from(error).with_context("persist cherry-pick mainline", Some(&path))
            })
    }

    fn persisted_cherry_pick_mainline_parent(&self, source: &str) -> Option<String> {
        let repo = self.repo();
        let contents = fs::read_to_string(repo.path().join(PERSISTED_CHERRY_PICK_MAINLINE)).ok()?;
        let mut lines = contents.lines();
        let persisted_source = lines.next()?;
        let parent = lines.next()?;
        if persisted_source.eq_ignore_ascii_case(source)
            && !parent.is_empty()
            && lines.next().is_none()
        {
            Some(parent.to_ascii_lowercase())
        } else {
            None
        }
    }

    fn clear_persisted_cherry_pick_mainline(&self) {
        let repo = self.repo();
        for name in [
            PERSISTED_CHERRY_PICK_MAINLINE,
            PERSISTED_CHERRY_PICK_MAINLINE_STAGING,
        ] {
            match fs::remove_file(repo.path().join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }

    /// The remaining steps in the sequencer todo plus the commit the sequence
    /// is stopped on; `None` without either. A single pick or revert writes no
    /// `sequencer` directory, only its `*_HEAD`.
    fn sequencer_progress_marker(&self, kind: SequenceKind) -> Result<Option<SequencerProgress>> {
        let repo = self.repo();
        let git_dir = repo.path();
        let remaining_steps = fs::read_to_string(git_dir.join("sequencer").join("todo"))
            .ok()
            .map(|todo| {
                todo.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .count()
            });
        let stopped_on = crate::refs::root_ref(&repo, kind.root_ref())?.map(|id| id.to_string());
        Ok(if remaining_steps.is_none() && stopped_on.is_none() {
            None
        } else {
            Some(SequencerProgress {
                remaining_steps,
                stopped_on,
            })
        })
    }

    /// Whether the index holds unmerged (conflict) entries — the signature
    /// of a rebase genuinely paused at a conflict.
    pub(super) fn index_has_conflicts(&self) -> bool {
        // Read fresh: the cached index may predate a just-written conflict
        // on a filesystem whose timestamps are too coarse to show the rewrite.
        self.repo()
            .open_index()
            .is_ok_and(|index| index.entries().iter().any(|e| e.stage_raw() != 0))
    }

    pub(super) fn rebase_abort_with_output_impl(&self) -> Result<CommandOutput> {
        if self.sequencer_state_impl()? == SequencerState::Revert {
            // Without REVERT_HEAD there is no stopped step to roll back: git
            // clears the sequence and leaves HEAD where the user put it.
            let stopped = self.revert_head_exists()?;
            let mut cmd = self.git_workdir_cmd();
            cmd.arg("revert").arg("--abort");
            let mut output = run_git_with_output(cmd, "git revert --abort")?;
            if !stopped {
                output.stdout = REVERT_ABORT_KEPT_HEAD_SENTINEL.to_string();
            }
            return Ok(output);
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--abort");
        match run_git_with_output(cmd, "git rebase --abort") {
            Ok(output) => {
                self.clear_persisted_cherry_pick_mainline();
                Ok(output)
            }
            Err(rebase_error) => {
                let mut cherry_pick_cmd = self.git_workdir_cmd();
                cherry_pick_cmd.arg("cherry-pick").arg("--abort");
                match run_git_with_output(cherry_pick_cmd, "git cherry-pick --abort") {
                    Ok(output) => {
                        self.clear_persisted_cherry_pick_mainline();
                        return Ok(output);
                    }
                    // When cherry-pick state remains on disk, this is the
                    // operation the user actually tried to abort. Preserve
                    // its actionable error instead of replacing it with the
                    // earlier "no rebase" failure after the `git am`
                    // fallback also fails.
                    Err(cherry_pick_error) if self.cherry_pick_in_progress_impl()? => {
                        return Err(cherry_pick_error);
                    }
                    Err(_) => {}
                }
                // `git am` uses its own sequencer state. Falling back here allows a
                // single "abort in-progress operation" UI action to handle both rebase
                // and patch-apply flows.
                let mut am_cmd = self.git_workdir_cmd();
                am_cmd.arg("am").arg("--abort");
                match run_git_with_output(am_cmd, "git am --abort") {
                    Ok(output) => Ok(output),
                    Err(_) => Err(rebase_error),
                }
            }
        }
    }

    pub(super) fn merge_abort_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge").arg("--abort");
        run_git_with_output(cmd, "git merge --abort")
    }

    pub(super) fn sequencer_state_impl(&self) -> Result<SequencerState> {
        let repo = self.repo();
        let state = match crate::refs::operation_state(&repo)? {
            Some(
                gix::state::InProgress::Rebase
                | gix::state::InProgress::RebaseInteractive
                | gix::state::InProgress::ApplyMailbox
                | gix::state::InProgress::ApplyMailboxRebase,
            ) => SequencerState::RebaseOrApply,
            Some(
                gix::state::InProgress::CherryPick | gix::state::InProgress::CherryPickSequence,
            ) => SequencerState::CherryPick,
            Some(gix::state::InProgress::Revert | gix::state::InProgress::RevertSequence) => {
                SequencerState::Revert
            }
            Some(gix::state::InProgress::Bisect) if self.revert_head_exists()? => {
                SequencerState::Revert
            }
            Some(gix::state::InProgress::Bisect) | None => self.leftover_sequence_state(),
            Some(gix::state::InProgress::Merge) => SequencerState::None,
        };
        if state != SequencerState::CherryPick {
            self.clear_persisted_cherry_pick_mainline();
        }
        Ok(state)
    }

    pub(super) fn rebase_in_progress_impl(&self) -> Result<bool> {
        Ok(self.sequencer_state_impl()? != SequencerState::None)
    }

    fn cherry_pick_in_progress_impl(&self) -> Result<bool> {
        let repo = self.repo();
        Ok(matches!(
            crate::refs::operation_state(&repo)?,
            Some(gix::state::InProgress::CherryPick | gix::state::InProgress::CherryPickSequence)
        ))
    }

    pub(super) fn interactive_cherry_pick_with_output_impl(
        &self,
        entries: &[InteractiveRebaseEntry],
        commit: bool,
    ) -> Result<CommandOutput> {
        if entries.is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "cherry-pick: no commits selected".to_string(),
            )));
        }
        for entry in entries {
            validate_hex_commit_id(&CommitId(entry.commit_id.clone().into()))?;
        }

        // For a cherry-pick, `drop` means "leave this commit out", which is
        // exactly the same as not listing it — so resolve it here rather than
        // emitting a `drop` line. Not merely tidier: `git rebase -i` without
        // `--rebase-merges` cannot represent a merge commit in its todo at
        // all, and a `drop <merge>` line makes its sequencer abort the whole
        // plan with "BUG: sequencer.c: unexpected todo_command" instead of
        // ignoring the line. Removing dropped entries keeps a dropped merge
        // from ever reaching git, and often leaves a pure-pick plan that takes
        // the plain `git cherry-pick` path below.
        let planned: Vec<InteractiveRebaseEntry> = entries
            .iter()
            .filter(|entry| entry.action != InteractiveRebaseAction::Drop)
            .cloned()
            .collect();
        if planned.is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "cherry-pick: every selected commit was dropped; nothing to apply".to_string(),
            )));
        }
        let entries = planned.as_slice();

        // `git cherry-pick` refuses a merge commit without `-m`, but only
        // when the sequence reaches it: earlier picks are already committed
        // by then, and that mid-sequence stop leaves sequencer state the UI
        // can neither continue nor abort. GitComet plans no mainline
        // selection, so reject merges before launching any step.
        let repo = self.repo();
        for entry in entries {
            let commit = peel_commit(&repo, &entry.commit_id)?;
            if commit.parent_ids().count() > 1 {
                let short = entry.commit_id.get(..8).unwrap_or(&entry.commit_id);
                return Err(Error::new(ErrorKind::Backend(format!(
                    "cherry-pick: {short} is a merge commit; multi-commit cherry-pick does not \
                     support merge commits — cherry-pick it individually and choose a mainline \
                     parent"
                ))));
            }
        }

        if !commit {
            return self.cherry_pick_uncommitted_sequence(entries);
        }
        // Git reports this only after writing `sequencer/` for the plan.
        if !self.index_matches_head()? {
            return Err(cherry_pick_error(
                "the index has staged changes; commit or unstage them first, or keep the \
                 cherry-picked changes uncommitted",
            ));
        }

        let pure_pick = entries
            .iter()
            .all(|entry| entry.action == InteractiveRebaseAction::Pick);
        if pure_pick {
            let mut cmd = self.git_workdir_cmd();
            // `--allow-empty` preserves source commits that were created
            // empty on purpose; without it they stop the sequence with the
            // same "now empty" status as already-applied picks and the
            // auto-skip below would silently drop them.
            cmd.arg("cherry-pick")
                .arg("--no-edit")
                .arg("--allow-empty")
                .arg("--");
            for entry in entries {
                cmd.arg(&entry.commit_id);
            }
            let label = format!("git cherry-pick {} commits", entries.len());
            // An already-applied commit stops the whole sequence with an
            // "empty" error that only `--skip` moves past, and the UI
            // exposes no skip control — advance automatically so the
            // remaining picks land.
            return self.run_sequence_step_output(SequenceKind::CherryPick, cmd, &label);
        }

        // Reject plans git's todo parser would reject only after starting
        // the rebase, and empty reword messages it would accept.
        let mut seen_commit_step = false;
        for entry in entries {
            match entry.action {
                // Already resolved away above; the arm only keeps the match
                // exhaustive.
                InteractiveRebaseAction::Drop => {}
                InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup => {
                    if !seen_commit_step {
                        return Err(Error::new(ErrorKind::Backend(
                            "cherry-pick: squash needs a previous picked commit".to_string(),
                        )));
                    }
                }
                InteractiveRebaseAction::Pick => seen_commit_step = true,
                InteractiveRebaseAction::Reword => {
                    if entry
                        .new_message
                        .as_ref()
                        .is_some_and(|message| message.trim().is_empty())
                    {
                        return Err(Error::new(ErrorKind::Backend(
                            "cherry-pick: reword message must not be empty".to_string(),
                        )));
                    }
                    seen_commit_step = true;
                }
            }
        }

        // The rebase machinery below needs HEAD as its upstream; on an
        // unborn branch git only says "invalid upstream 'HEAD'", so name
        // the actual limitation. Plain multi-picks work there (git can
        // cherry-pick root commits onto an unborn branch), and a plan whose
        // only non-pick steps were drops already took that path above.
        if gix_head_id_or_none(&repo)?.is_none() {
            return Err(Error::new(ErrorKind::Backend(
                "cherry-pick: reword and squash plans need an existing commit on the \
                 current branch; pick the commits unmodified first, or make an initial commit"
                    .to_string(),
            )));
        }

        // Plans with reword/squash/fixup/drop steps run through `git rebase
        // -i HEAD` with the planned todo installed (the upstream is HEAD, so
        // git's own todo starts empty and the installed picks are pure
        // additions). Git's sequencer then owns the plan: a conflict pauses
        // with the remaining steps — and any reword messages, via the
        // persisted state — on disk for the regular continue path, and
        // rebase refuses to start over a dirty index or worktree instead of
        // folding unrelated staged changes into the picked commits.
        let label = format!("git cherry-pick --interactive {} commits", entries.len());
        self.run_planned_rebase(entries, "HEAD", &label)
    }

    /// Picks `entries` one at a time with `--no-commit`, each merging into
    /// the index like `cherry-pick -n`. No sequencer state is written:
    /// a stop leaves only the conflicts, with no Abort that could reset away
    /// the staged work, and names the commits still to pick.
    fn cherry_pick_uncommitted_sequence(
        &self,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        if entries
            .iter()
            .any(|entry| entry.action != InteractiveRebaseAction::Pick)
        {
            return Err(cherry_pick_error(
                "reword, squash and fixup steps need commits; change them to pick, or commit \
                 the cherry-picked commits",
            ));
        }
        if let Some(operation) = self.operation_in_progress_label()? {
            return Err(cherry_pick_error(&format!(
                "{operation} is in progress; finish or abort it first"
            )));
        }
        if self.index_has_conflicts() {
            return Err(cherry_pick_error(
                "the index has unresolved conflicts; resolve them first",
            ));
        }

        let mut output = CommandOutput::empty_success(format!(
            "git cherry-pick --no-commit {} commits",
            entries.len()
        ));
        let mut tree = self.index_tree()?;
        let mut changed_any = false;
        for (ix, entry) in entries.iter().enumerate() {
            let label = format!("git cherry-pick --no-commit {}", entry.commit_id);
            let mut cmd = self.git_workdir_cmd();
            cmd.args(["cherry-pick", "--no-commit", "--"])
                .arg(&entry.commit_id);
            let step = run_git_raw_output(cmd, &label)?;
            append_raw_output(&mut output, &step);
            if !step.status.success() {
                return Err(self.uncommitted_sequence_stopped(entries, ix, &label, step));
            }
            let next = self.index_tree()?;
            changed_any |= next != tree;
            tree = next;
        }
        if !changed_any {
            output.stdout = CHERRY_PICK_ALREADY_APPLIED_SENTINEL.to_string();
        }
        Ok(output)
    }

    fn index_tree(&self) -> Result<String> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("write-tree");
        Ok(run_git_capture(cmd, "git write-tree")?.trim().to_string())
    }

    /// The error for an uncommitted sequence that stopped at `entries[stopped]`,
    /// saying what was applied and what is left to pick.
    fn uncommitted_sequence_stopped(
        &self,
        entries: &[InteractiveRebaseEntry],
        stopped: usize,
        label: &str,
        output: crate::util::GitOutput,
    ) -> Error {
        let short = |entry: &InteractiveRebaseEntry| short_commit_id(&entry.commit_id).to_string();
        let conflicted = self.index_has_conflicts();
        let mut detail = format!(
            "Applied {stopped} of {} commits without committing; {} {}.",
            entries.len(),
            short(&entries[stopped]),
            if conflicted {
                "conflicts"
            } else {
                "could not be applied"
            }
        );
        let remaining: Vec<String> = entries[stopped + 1..].iter().map(short).collect();
        match (conflicted, remaining.is_empty()) {
            (true, true) => detail.push_str(" Resolve the conflicts to finish."),
            (true, false) => {
                let _ = write!(
                    detail,
                    " Resolve the conflicts, then cherry-pick the remaining commits: {}.",
                    remaining.join(", ")
                );
            }
            (false, true) => {}
            (false, false) => {
                let _ = write!(detail, " Not yet picked: {}.", remaining.join(", "));
            }
        }
        let git_said = bytes_to_text_preserving_utf8(&output.stderr);
        if !git_said.trim().is_empty() {
            let _ = write!(detail, "\n\n{}", git_said.trim());
        }
        Error::new(ErrorKind::Git(
            GitFailure::new(
                label,
                GitFailureId::CommandFailed,
                output.status.code(),
                output.stdout,
                output.stderr,
                Some(detail),
            )
            .with_auth_outcome(output.auth_outcome),
        ))
    }

    pub(super) fn merge_commit_message_impl(&self) -> Result<Option<String>> {
        if crate::refs::operation_state(&self.repo())? != Some(gix::state::InProgress::Merge) {
            return Ok(None);
        }
        self.commit_message_template_impl()
    }

    /// MERGE_MSG without its comment lines: the message git prepared for the
    /// next commit (a merge, or a `revert --no-commit`).
    pub(super) fn commit_message_template_impl(&self) -> Result<Option<String>> {
        let repo = self.repo();
        let merge_msg_path = repo.path().join("MERGE_MSG");
        let contents = match std::fs::read_to_string(&merge_msg_path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(
                    Error::from(e).with_context("read commit message", Some(&merge_msg_path))
                );
            }
        };

        let mut lines: Vec<&str> = Vec::new();
        for line in contents.lines() {
            let line = line.trim_end();
            if line.trim_start().starts_with('#') {
                continue;
            }
            lines.push(line);
        }

        let Some(start) = lines.iter().position(|l| !l.trim().is_empty()) else {
            return Ok(None);
        };
        let end = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|ix| ix + 1)
            .unwrap_or(start + 1);

        let message = lines[start..end].join("\n");
        if message.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(message))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conflicts_are_read_from_a_fresh_index_when_its_mtime_does_not_move() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let git = |args: &[&str]| {
            crate::test_support::git_command()
                .current_dir(path)
                .args(["-c", "user.name=T", "-c", "user.email=t@e.st"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "base\n").unwrap();
        git(&["add", "a.txt"]);
        git(&["commit", "-qm", "base"]);
        git(&["checkout", "-qb", "dev"]);
        std::fs::write(path.join("a.txt"), "dev\n").unwrap();
        git(&["commit", "-qam", "dev"]);
        git(&["checkout", "-q", "main"]);
        std::fs::write(path.join("a.txt"), "main\n").unwrap();
        git(&["commit", "-qam", "main"]);

        let repo = crate::test_support::open_repo(path);
        // Cache the pre-merge index in gix's shared snapshot.
        assert!(repo.repo().index_or_empty().is_ok());
        assert!(!repo.index_has_conflicts());
        let index = path.join(".git/index");
        let cached_mtime = std::fs::metadata(&index).unwrap().modified().unwrap();
        assert!(!git(&["merge", "dev"]).status.success());
        // A coarse-timestamp filesystem: the rewritten index keeps its mtime.
        std::fs::File::options()
            .write(true)
            .open(&index)
            .unwrap()
            .set_modified(cached_mtime)
            .unwrap();
        assert!(repo.index_has_conflicts());
    }
}
