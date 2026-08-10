use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_capture, run_git_raw_output,
    run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, InteractiveRebaseAction, InteractiveRebaseEntry, ResetMode, Result,
    SequencerState,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Returns the HEAD commit id, or `None` when HEAD is unborn / empty.
pub(super) fn gix_head_id_or_none(repo: &gix::Repository) -> Result<Option<gix::ObjectId>> {
    let mut head = repo
        .head()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;
    head.try_peel_to_id()
        .map(|id| id.map(|id| id.detach()))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head peel: {e}"))))
}

/// Upper bound on the number of commits a single squash may cover; a runaway
/// walk past this means `oldest` is not a first-parent ancestor of `head`.
const MAX_SQUASH_CHAIN: usize = 10_000;
const PERSISTED_REWORD_DIR: &str = "gitcomet-reword";
const PERSISTED_REWORD_STAGING_DIR: &str = "gitcomet-reword.staging";
const PERSISTED_PLAN_NAME: &str = "planned-todo";
const PERSISTED_CHERRY_PICK_MAINLINE: &str = "gitcomet-cherry-pick-mainline";
const PERSISTED_CHERRY_PICK_MAINLINE_STAGING: &str = "gitcomet-cherry-pick-mainline.staging";

// A POSIX sh script on every platform: Git for Windows also runs editors
// through its bundled `sh`, and a shebang script survives a script path
// containing spaces — a batch file does not, because the MSYS runtime
// re-spawns it via `cmd.exe /c` with the path unquoted.
const MSG_EDITOR_NAME: &str = "gitcomet-msg-editor.sh";

fn peel_commit<'r>(repo: &'r gix::Repository, spec: &str) -> Result<gix::Commit<'r>> {
    repo.rev_parse_single(spec)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
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

fn append_command_output(acc: &mut CommandOutput, output: CommandOutput) {
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

fn append_raw_output(acc: &mut CommandOutput, output: &std::process::Output) {
    append_command_output(
        acc,
        CommandOutput {
            command: String::new(),
            stdout: bytes_to_text_preserving_utf8(&output.stdout),
            stderr: bytes_to_text_preserving_utf8(&output.stderr),
            exit_code: output.status.code(),
        },
    );
}

/// On-disk position of an in-progress cherry-pick, compared before and after
/// a continue to tell "advanced and paused at a later step" from "failed in
/// place".
#[derive(PartialEq)]
struct CherryPickProgress {
    /// Steps left in `sequencer/todo`; `None` for a single-commit
    /// cherry-pick, which keeps no todo.
    remaining_steps: Option<usize>,
    /// `CHERRY_PICK_HEAD` — the commit the sequence is stopped on.
    stopped_on: Option<String>,
}

impl CherryPickProgress {
    fn advanced_from(&self, before: &CherryPickProgress) -> bool {
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

        let repo = self._repo.to_thread_local();
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

        // Validate mainline selection before invoking git so a stale or
        // malformed UI request cannot leave cherry-pick state behind.
        let repo = self._repo.to_thread_local();
        let parent_ids = peel_commit(&repo, id.as_ref())?
            .parent_ids()
            .map(|parent| parent.detach().to_string())
            .collect::<Vec<_>>();
        let parent_count = parent_ids.len();
        let short = id.as_ref().get(..8).unwrap_or(id.as_ref());
        match (parent_count > 1, mainline) {
            (true, None) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "cherry-pick: {short} is a merge commit with {parent_count} parents; choose a \
                     mainline parent"
                ))));
            }
            (true, Some(parent)) if parent == 0 || parent > parent_count => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "cherry-pick: mainline parent {parent} is invalid for merge commit {short}; \
                     choose a parent from 1 to {parent_count}"
                ))));
            }
            (false, Some(_)) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "cherry-pick: {short} is not a merge commit; a mainline parent cannot be \
                     selected"
                ))));
            }
            _ => {}
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
        let mainline_label = mainline.map_or_else(String::new, |parent| format!(" -m {parent}"));
        let label = if commit {
            format!("git cherry-pick{mainline_label} {}", id.as_ref())
        } else {
            format!(
                "git cherry-pick{mainline_label} --no-commit {}",
                id.as_ref()
            )
        };

        let output = run_git_raw_output(cmd, &label)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("failed to run {label}: {e}"))))?;
        if output.status.success() {
            if !started_with_sequencer {
                self.clear_persisted_cherry_pick_mainline();
            }
            return Ok(CommandOutput {
                command: label,
                stdout: bytes_to_text_preserving_utf8(&output.stdout),
                stderr: bytes_to_text_preserving_utf8(&output.stderr),
                exit_code: output.status.code(),
            });
        }

        if self.cherry_pick_stopped_became_empty()? {
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

    pub(super) fn rebase_continue_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let repo = self._repo.to_thread_local();
        match persisted_reword_state(repo.path()) {
            PersistedReword::Ready {
                editor,
                messages,
                plan,
            } => {
                // The persisted editor only services steps the plan knew
                // about; a message-editing step added to the todo outside
                // GitComet would silently keep its old message.
                if rebase_unplanned_message_edit(repo.path(), &plan) {
                    return Err(Error::new(ErrorKind::Backend(
                        "this rebase has pending reword/squash steps that GitComet did not \
                         plan; continue it in a terminal with `git rebase --continue`, or \
                         abort the rebase"
                            .to_string(),
                    )));
                }
                cmd.env("GIT_EDITOR", shell_quote_path(&editor));
                cmd.env("GITCOMET_MSGS_DIR", messages);
                cmd.env("GITCOMET_GIT_DIR", repo.path());
            }
            PersistedReword::Damaged => {
                return Err(Error::new(ErrorKind::Backend(
                    "GitComet's reword data for this rebase is incomplete; continue it in a \
                     terminal with `git rebase --continue`, or abort the rebase"
                        .to_string(),
                )));
            }
            PersistedReword::Absent => {
                // `git rebase --continue` may open an editor to confirm the
                // replayed commit's message. A GUI subprocess has no terminal
                // to service it, so a no-op editor is used — but only when no
                // message-editing step is pending: silently accepting a
                // pending reword/squash message would finalize rewritten
                // history the user still meant to edit (e.g. a rebase started
                // in a terminal, continued here after a conflict).
                if rebase_pending_message_edit(repo.path()) {
                    return Err(Error::new(ErrorKind::Backend(
                        "this rebase has pending reword/squash steps that GitComet did not \
                         plan; continue it in a terminal with `git rebase --continue`, or \
                         abort the rebase"
                            .to_string(),
                    )));
                }
                cmd.env("GIT_EDITOR", "true");
            }
        }
        cmd.arg("rebase").arg("--continue");
        match self.run_rebase_step_output(cmd, "git rebase --continue") {
            Ok(output) => Ok(output),
            Err(rebase_error) => {
                let mut cherry_pick_cmd = self.git_workdir_cmd();
                cherry_pick_cmd.arg("cherry-pick").arg("--continue");
                match self
                    .run_cherry_pick_step_output(cherry_pick_cmd, "git cherry-pick --continue")
                {
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

    /// Run a rebase step (`rebase -i` / `rebase --continue`). A non-zero exit
    /// can be a normal outcome: git advanced and paused at the next conflict
    /// with the rebase left in progress. That case — and only that case — is
    /// reported as success (with the captured output) so the UI treats it as
    /// "paused at conflict": it clears the loading state, reloads status, and
    /// surfaces the new conflict. A non-zero exit that made no sequencer
    /// progress (unresolved conflicts on continue, hook/signing/editor
    /// failures) keeps the original git error — as does an initial command
    /// that progressed but left no conflict behind: GitComet plans only
    /// pick/reword/squash/fixup/drop steps, so a conflict is the only
    /// legitimate reason the initial `rebase -i` stops non-zero, and
    /// anything else (a failing hook or signer) must keep git's message.
    fn run_rebase_step_output(&self, cmd: Command, label: &str) -> Result<CommandOutput> {
        let steps_before = self.rebase_progress_marker();
        let output = run_git_raw_output(cmd, label)?;
        let paused_after_progress = !output.status.success()
            && self.rebase_in_progress_impl()?
            && match (steps_before, self.rebase_progress_marker()) {
                // The command started the rebase and paused at a conflict.
                (None, Some(_)) => self.index_has_conflicts(),
                // The command advanced past the step it was stuck on. No
                // conflict requirement here: a continue can legitimately
                // stop without one (a failing `exec` in an externally
                // planned todo).
                (Some(before), Some(after)) => after > before,
                (_, None) => false,
            };
        if output.status.success() || paused_after_progress {
            Ok(CommandOutput {
                command: label.to_string(),
                stdout: bytes_to_text_preserving_utf8(&output.stdout),
                stderr: bytes_to_text_preserving_utf8(&output.stderr),
                exit_code: output.status.code(),
            })
        } else {
            Err(git_command_failed_error(label, output))
        }
    }

    /// Run a cherry-pick step (`cherry-pick --continue`). Empty stops are
    /// advanced past automatically ([`Self::run_cherry_pick_auto_skip`]).
    /// Like the rebase path, a non-zero exit is reported as success only
    /// when the sequencer genuinely advanced and paused again at a later
    /// step; a continue that made no progress (unresolved conflicts, a
    /// failed hook re-running the same step) keeps git's error.
    fn run_cherry_pick_step_output(&self, cmd: Command, label: &str) -> Result<CommandOutput> {
        let marker_before = self.cherry_pick_progress_marker();
        let (output, last) = self.run_cherry_pick_auto_skip(cmd, label)?;
        let still_in_progress = self.cherry_pick_in_progress_impl()?;
        if !still_in_progress {
            self.clear_persisted_cherry_pick_mainline();
        }
        if last.status.success() {
            return Ok(output);
        }
        let paused_after_progress = still_in_progress
            && match (marker_before, self.cherry_pick_progress_marker()) {
                // The command started a cherry-pick and paused at a conflict.
                (None, Some(_)) => self.index_has_conflicts(),
                // Native cherry-pick todos contain only pick steps. If Git
                // advanced and then stopped without an unmerged index, the
                // later command failed (for example a hook or signer); that
                // is not a conflict pause and its error must be preserved.
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

    /// Runs a cherry-pick command and, whenever it stops because the current
    /// pick is empty (its changes already applied), advances the sequence
    /// with `git cherry-pick --skip`: the UI exposes only continue and
    /// abort, and `cherry-pick --continue` refuses an empty step, so without
    /// this the remaining picks would need a terminal. Returns the
    /// accumulated output of every command run and the last command's raw
    /// output for status inspection.
    fn run_cherry_pick_auto_skip(
        &self,
        cmd: Command,
        label: &str,
    ) -> Result<(CommandOutput, std::process::Output)> {
        let mut acc = CommandOutput {
            command: label.to_string(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
        };
        let mut last = run_git_raw_output(cmd, label)?;
        append_raw_output(&mut acc, &last);
        while !last.status.success() && self.cherry_pick_stopped_became_empty()? {
            let marker = self.cherry_pick_progress_marker();
            let mut skip = self.git_workdir_cmd();
            skip.arg("cherry-pick").arg("--skip");
            last = run_git_raw_output(skip, "git cherry-pick --skip")?;
            append_raw_output(&mut acc, &last);
            // A skip that moved nothing forward would loop on the same
            // step's output forever; surface it instead.
            if self.cherry_pick_progress_marker() == marker {
                break;
            }
        }
        acc.exit_code = last.status.code();
        Ok((acc, last))
    }

    /// Whether a stopped cherry-pick's source had changes but applying them
    /// produced an empty index. An intentionally empty source also leaves
    /// `CHERRY_PICK_HEAD`, no conflicts, and a clean index when commit
    /// creation fails (for example in a hook or signer), so checking only
    /// repository cleanliness would silently skip that real failure.
    fn cherry_pick_stopped_became_empty(&self) -> Result<bool> {
        if !self.cherry_pick_in_progress_impl()? || self.index_has_conflicts() {
            return Ok(false);
        }
        let Some(stopped_on) = self
            .cherry_pick_progress_marker()
            .and_then(|progress| progress.stopped_on)
        else {
            return Ok(false);
        };

        // Compare against an explicit parent. Without one, `diff-tree` emits
        // no merge diff and incorrectly classifies every merge as empty.
        // GitComet persists the chosen parent for merge picks it starts; an
        // external merge pick without that metadata is deliberately not
        // auto-skipped because guessing the mainline could hide a real
        // signing/hook failure.
        let repo = self._repo.to_thread_local();
        let parents = peel_commit(&repo, &stopped_on)?
            .parent_ids()
            .map(|parent| parent.detach().to_string())
            .collect::<Vec<_>>();
        let source_parent = match parents.as_slice() {
            [] => None,
            [parent] => Some(parent.clone()),
            _ => match self.persisted_cherry_pick_mainline_parent(&stopped_on) {
                Some(parent) if parents.contains(&parent) => Some(parent),
                _ => return Ok(false),
            },
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

        let mut cmd = self.git_workdir_cmd();
        cmd.args(["diff", "--cached", "--quiet"]);
        let output = run_git_raw_output(cmd, "git diff --cached --quiet")?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_command_failed_error(
                "git diff --cached --quiet",
                output,
            )),
        }
    }

    fn persist_cherry_pick_mainline(&self, source: &str, parent: &str) -> Result<()> {
        let repo = self._repo.to_thread_local();
        let path = repo.path().join(PERSISTED_CHERRY_PICK_MAINLINE);
        let staging = repo.path().join(PERSISTED_CHERRY_PICK_MAINLINE_STAGING);
        fs::write(&staging, format!("{source}\n{parent}\n"))
            .and_then(|()| {
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                fs::rename(&staging, &path)
            })
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))
    }

    fn persisted_cherry_pick_mainline_parent(&self, source: &str) -> Option<String> {
        let repo = self._repo.to_thread_local();
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
        let repo = self._repo.to_thread_local();
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

    /// Progress marker of the cherry-pick in progress: the remaining steps
    /// in the sequencer todo plus the commit the sequence is stopped on.
    /// `None` when no cherry-pick state exists at all. A single-commit
    /// cherry-pick writes no `sequencer` directory, only `CHERRY_PICK_HEAD`.
    fn cherry_pick_progress_marker(&self) -> Option<CherryPickProgress> {
        let repo = self._repo.to_thread_local();
        let git_dir = repo.path();
        let remaining_steps = fs::read_to_string(git_dir.join("sequencer").join("todo"))
            .ok()
            .map(|todo| {
                todo.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .count()
            });
        let stopped_on = fs::read_to_string(git_dir.join("CHERRY_PICK_HEAD"))
            .ok()
            .map(|sha| sha.trim().to_string());
        if remaining_steps.is_none() && stopped_on.is_none() {
            None
        } else {
            Some(CherryPickProgress {
                remaining_steps,
                stopped_on,
            })
        }
    }

    /// Whether the index holds unmerged (conflict) entries — the signature
    /// of a rebase genuinely paused at a conflict.
    fn index_has_conflicts(&self) -> bool {
        let repo = self._repo.to_thread_local();
        repo.index_or_empty()
            .is_ok_and(|index| index.entries().iter().any(|e| e.stage_raw() != 0))
    }

    /// Completed-step count of the rebase in progress, from the merge
    /// backend's `done` file or the apply backend's `next` counter. `None`
    /// when no rebase state exists.
    fn rebase_progress_marker(&self) -> Option<usize> {
        let repo = self._repo.to_thread_local();
        let git_dir = repo.path();
        if let Ok(done) = fs::read_to_string(git_dir.join("rebase-merge").join("done")) {
            return Some(done.lines().count());
        }
        fs::read_to_string(git_dir.join("rebase-apply").join("next"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub(super) fn rebase_abort_with_output_impl(&self) -> Result<CommandOutput> {
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
        let repo = self._repo.to_thread_local();
        let state = match repo.state() {
            Some(
                gix::state::InProgress::Rebase
                | gix::state::InProgress::RebaseInteractive
                | gix::state::InProgress::ApplyMailbox
                | gix::state::InProgress::ApplyMailboxRebase,
            ) => SequencerState::RebaseOrApply,
            Some(
                gix::state::InProgress::CherryPick | gix::state::InProgress::CherryPickSequence,
            ) => SequencerState::CherryPick,
            _ => SequencerState::None,
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
        let repo = self._repo.to_thread_local();
        Ok(matches!(
            repo.state(),
            Some(gix::state::InProgress::CherryPick | gix::state::InProgress::CherryPickSequence)
        ))
    }

    pub(super) fn list_commits_for_interactive_rebase_impl(
        &self,
        base: &str,
    ) -> Result<Vec<InteractiveRebaseEntry>> {
        validate_ref_like_arg(base, "interactive rebase base")?;

        let range = format!("{base}..HEAD");
        let mut cmd = self.git_workdir_cmd();
        // NUL-framed fields (sha, subject, full message): commit messages are
        // NUL-free by construction, so unlike other control bytes NUL cannot
        // collide with message content. `-z` NUL-terminates each record.
        //
        // The listing must match the flattened todo `git rebase -i` itself
        // generates: `--no-merges` because `pick` rejects merge commits (the
        // merge is linearized away, like plain git rebase), and
        // `--topo-order` so a parent can never sort after its child (date
        // order allows that under clock skew, which would make the installed
        // todo unreplayable).
        cmd.args([
            "log",
            "-z",
            "--format=%H%x00%s%x00%B",
            "--reverse",
            "--topo-order",
            "--no-merges",
            &range,
        ]);
        let output = run_git_capture(cmd, &format!("git log {range}"))?;

        parse_interactive_rebase_log(&output)
    }

    pub(super) fn interactive_rebase_with_output_impl(
        &self,
        base: &str,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(base, "interactive rebase base")?;

        // The plan was made from a possibly stale snapshot. Re-list the live
        // range before spawning git: the installed todo replaces git's own,
        // so a commit added (or history rewritten) since setup would
        // otherwise be silently dropped from the branch. Order is not
        // compared — reordering entries is part of the feature.
        let live = self.list_commits_for_interactive_rebase_impl(base)?;
        let mut live_ids: Vec<&str> = live.iter().map(|e| e.commit_id.as_str()).collect();
        let mut planned_ids: Vec<&str> = entries.iter().map(|e| e.commit_id.as_str()).collect();
        live_ids.sort_unstable();
        planned_ids.sort_unstable();
        if live_ids != planned_ids {
            return Err(Error::new(ErrorKind::Backend(
                "interactive rebase aborted: the branch changed since the rebase was set \
                 up; reload the setup and try again"
                    .to_string(),
            )));
        }

        let label = format!("git rebase -i {base}");
        self.run_planned_rebase(entries, base, &label)
    }

    /// Runs `git rebase -i <upstream>` with the planned todo installed via
    /// the sequence editor (and the message editor when the plan rewords or
    /// squashes), keeping reword data alongside git's state if the rebase
    /// pauses so a later continue can service it. `--empty=drop` drops picks
    /// whose changes are already upstream: git would otherwise stop on them,
    /// and the UI exposes no skip control to move past that stop. Commits
    /// that started out empty are unaffected and stay preserved.
    fn run_planned_rebase(
        &self,
        entries: &[InteractiveRebaseEntry],
        upstream: &str,
        label: &str,
    ) -> Result<CommandOutput> {
        let scripts = RebaseScripts::create(entries).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "failed to create rebase scripts: {e}"
            )))
        })?;

        let mut cmd = self.git_workdir_cmd();
        cmd.env(
            "GIT_SEQUENCE_EDITOR",
            shell_quote_path(&scripts.seq_editor_path),
        );
        if let Some(ref msg_editor) = scripts.msg_editor_path {
            cmd.env("GIT_EDITOR", shell_quote_path(msg_editor));
            cmd.env("GITCOMET_MSGS_DIR", &scripts.msgs_dir);
            let repo = self._repo.to_thread_local();
            cmd.env("GITCOMET_GIT_DIR", repo.path());
        }
        cmd.env("GITCOMET_TODO_FILE", &scripts.todo_path);
        let empty_policy = if entries.iter().any(|entry| {
            matches!(
                entry.action,
                InteractiveRebaseAction::Squash | InteractiveRebaseAction::Fixup
            )
        }) {
            // Dropping a newly-empty fold anchor makes Git attach the
            // following squash/fixup to the commit that preceded the plan —
            // potentially rewriting an unrelated target commit. Keeping all
            // newly-empty picks in a fold-containing plan is the safe,
            // deterministic policy.
            "--empty=keep"
        } else {
            "--empty=drop"
        };
        cmd.args(["rebase", "-i", empty_policy]);
        cmd.arg("--").arg(upstream);

        let mut result = self.run_rebase_step_output(cmd, label);
        if self.rebase_in_progress_impl()? {
            let repo = self._repo.to_thread_local();
            // The rebase started and git's state is still on disk — whether
            // paused at a conflict (Ok) or stopped by a non-conflict failure
            // (Err, e.g. a broken signer): keep the planned messages with
            // that state either way so a later continue can service them.
            // A persist failure degrades to a warning on the Ok path — the
            // continue path refuses to finalize pending rewords without the
            // persisted state, so the messages cannot be silently lost.
            if let Err(e) = scripts.persist_reword_state(repo.path())
                && let Ok(output) = &mut result
            {
                if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
                    output.stderr.push('\n');
                }
                output.stderr.push_str(&format!(
                    "warning: failed to preserve interactive rebase messages: {e}\n"
                ));
            }
        }
        result
    }

    pub(super) fn interactive_cherry_pick_with_output_impl(
        &self,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        if entries.is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "cherry-pick: no commits selected".to_string(),
            )));
        }
        for entry in entries {
            validate_hex_commit_id(&CommitId(entry.commit_id.clone().into()))?;
        }

        // `git cherry-pick` refuses a merge commit without `-m`, but only
        // when the sequence reaches it: earlier picks are already committed
        // by then, and that mid-sequence stop leaves sequencer state the UI
        // can neither continue nor abort. GitComet plans no mainline
        // selection, so reject merges before launching any step.
        let repo = self._repo.to_thread_local();
        for entry in entries {
            if entry.action == InteractiveRebaseAction::Drop {
                continue;
            }
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
            return self.run_cherry_pick_step_output(cmd, &label);
        }

        // Reject plans git's todo parser would reject only after starting
        // the rebase, and empty reword messages it would accept.
        let mut seen_commit_step = false;
        for entry in entries {
            match entry.action {
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
        // cherry-pick root commits onto an unborn branch).
        if gix_head_id_or_none(&repo)?.is_none() {
            return Err(Error::new(ErrorKind::Backend(
                "cherry-pick: reword, squash, and drop plans need an existing commit on the \
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

<<<<<<< New base: Support explicit commit ranges when cherry-picking onto a new branch (#17)
    /// Creates `new_branch` at `base`'s tip, checks it out, and cherry-picks
    /// every commit reachable from `source` but not from `range` (oldest
    /// first, merge commits skipped) onto it. Nothing is created when the
    /// range is empty, `range` is not an ancestor of `source`, or
    /// `new_branch` already exists (`create_branch_impl` rejects it).
    pub(super) fn cherry_pick_range_onto_new_branch_impl(
        &self,
        base: &str,
        range: &str,
        source: &str,
        new_branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(base, "base branch name")?;
        validate_ref_like_arg(range, "range reference")?;
        validate_ref_like_arg(source, "source reference")?;
        validate_ref_like_arg(new_branch, "new branch name")?;

        // `range` must be an ancestor of `source` for `range..source` to be a
        // meaningful commit set; git's --is-ancestor exits 1 when it is not.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge-base").arg("--is-ancestor").arg(range).arg(source);
        let ancestor_label = format!("git merge-base --is-ancestor {range} {source}");
        match run_git_raw_output(cmd, &ancestor_label) {
            Ok(output) if output.status.success() => {}
            Ok(output) if output.status.code() == Some(1) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "{range} is not an ancestor of {source}; the range {range}..{source} would \
                     include unrelated history — pick a range reference that is an ancestor"
                ))));
            }
            Ok(output) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "failed to check whether {range} is an ancestor of {source}: {}",
                    bytes_to_text_preserving_utf8(&output.stderr).trim()
                ))));
            }
            Err(e) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "failed to check whether {range} is an ancestor of {source}: {e}"
                ))));
            }
        }

        // Oldest-first, merge commits skipped: the same set `git cherry-pick
        // range..source` would apply, enumerated explicitly so an empty range
        // is rejected before any branch or checkout is created.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-list")
            .arg("--reverse")
            .arg("--no-merges")
            .arg(format!("{range}..{source}"));
        let rev_list_label = format!("git rev-list --reverse --no-merges {range}..{source}");
        let output = run_git_raw_output(cmd, &rev_list_label).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "failed to list commits in {range}..{source}: {e}"
            )))
        })?;
        let shas: Vec<String> = bytes_to_text_preserving_utf8(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        if shas.is_empty() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "no commits to cherry-pick: {source} has no commits that are not already in \
                 {range}"
            ))));
        }

        // Create the branch from `base` (errors if it already exists) and
        // move onto it before the picks, mirroring create-branch-and-checkout.
        self.create_branch_impl(new_branch, &CommitId(base.into()))?;
        self.checkout_branch_impl(new_branch)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cherry-pick")
            .arg("--no-edit")
            .arg("--allow-empty")
            .arg("--");
        for sha in &shas {
            cmd.arg(sha);
        }
        let label = format!("git cherry-pick {} commits onto {new_branch}", shas.len());
        // An already-applied commit stops the whole sequence with an "empty"
        // error that only `--skip` moves past, and the UI exposes no skip
        // control — advance automatically so the remaining picks land.
        self.run_cherry_pick_step_output(cmd, &label)
    }

||||||| Common ancestor
=======
    /// Creates `new_branch` at `base`'s tip, checks it out, and cherry-picks
    /// every commit reachable from `source` but not from `base` (oldest first,
    /// merge commits skipped) onto it. Nothing is created when the range is
    /// empty, and `create_branch_impl` already rejects an existing branch.
    pub(super) fn cherry_pick_range_onto_new_branch_impl(
        &self,
        base: &str,
        source: &str,
        new_branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(base, "base branch name")?;
        validate_ref_like_arg(source, "source branch name")?;
        validate_ref_like_arg(new_branch, "new branch name")?;

        // Oldest-first, merge commits skipped: the same set `git cherry-pick
        // base..source` would apply, enumerated explicitly so an empty range
        // is rejected before any branch or checkout is created.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-list")
            .arg("--reverse")
            .arg("--no-merges")
            .arg(format!("{base}..{source}"));
        let rev_list_label = format!("git rev-list --reverse --no-merges {base}..{source}");
        let output = run_git_raw_output(cmd, &rev_list_label).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "failed to list commits in {base}..{source}: {e}"
            )))
        })?;
        let shas: Vec<String> = bytes_to_text_preserving_utf8(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        if shas.is_empty() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "no commits to cherry-pick: {source} has no commits that are not already in {base}"
            ))));
        }

        // Create the branch from `base` (errors if it already exists) and
        // move onto it before the picks, mirroring create-branch-and-checkout.
        self.create_branch_impl(new_branch, &CommitId(base.into()))?;
        self.checkout_branch_impl(new_branch)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cherry-pick")
            .arg("--no-edit")
            .arg("--allow-empty")
            .arg("--");
        for sha in &shas {
            cmd.arg(sha);
        }
        let label = format!("git cherry-pick {} commits onto {new_branch}", shas.len());
        // An already-applied commit stops the whole sequence with an "empty"
        // error that only `--skip` moves past, and the UI exposes no skip
        // control — advance automatically so the remaining picks land.
        self.run_cherry_pick_step_output(cmd, &label)
    }

>>>>>>> Current commit: Add cherry-pick branch A onto B as new branch C from the action bar
    pub(super) fn merge_commit_message_impl(&self) -> Result<Option<String>> {
        let repo = self._repo.to_thread_local();
        if repo.state() != Some(gix::state::InProgress::Merge) {
            return Ok(None);
        }

        let merge_msg_path = repo.path().join("MERGE_MSG");
        let contents = match std::fs::read_to_string(&merge_msg_path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::new(ErrorKind::Io(e.kind()))),
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

struct RebaseScripts {
    _dir: tempfile::TempDir,
    seq_editor_path: PathBuf,
    todo_path: PathBuf,
    msg_editor_path: Option<PathBuf>,
    msgs_dir: PathBuf,
}

impl RebaseScripts {
    fn create(entries: &[InteractiveRebaseEntry]) -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;

        let todo_content = build_todo_content(entries);
        let todo_path = dir.path().join("git-rebase-todo");
        fs::write(&todo_path, &todo_content)?;

        let seq_editor_path = dir.path().join("gitcomet-seq-editor.sh");
        fs::write(&seq_editor_path, seq_editor_script_contents())?;
        #[cfg(unix)]
        make_executable(&seq_editor_path)?;

        let msgs_dir = dir.path().join("msgs");
        let mut msg_editor_path = None;
        // Reword steps open an editor, and so does every fold run containing
        // a squash (at the run's last squash/fixup step) — even when no entry
        // is a reword. Without the no-op-capable editor installed, git would
        // launch the user's default editor in a TTY-less subprocess there.
        let needs_msg_editor = entries.iter().any(|e| {
            matches!(
                e.action,
                InteractiveRebaseAction::Reword | InteractiveRebaseAction::Squash
            )
        });
        if needs_msg_editor {
            fs::create_dir_all(&msgs_dir)?;
            for (ix, entry) in entries.iter().enumerate() {
                if entry.action == InteractiveRebaseAction::Reword
                    && let Some(ref msg) = entry.new_message
                {
                    // When commits squash into this entry, git builds the
                    // final message at the run's last squash/fixup step and
                    // would re-append the squashed messages over anything
                    // installed at the reword step — so the replacement
                    // message must be keyed to that step's commit instead.
                    let key_entry = gitcomet_core::squash::squash_run_final_entry(entries, ix)
                        .map_or(entry, |k| &entries[k]);
                    let msg_file = msgs_dir.join(&key_entry.commit_id);
                    fs::write(msg_file, msg.as_bytes())?;
                }
            }

            let path = dir.path().join(MSG_EDITOR_NAME);
            let script = msg_editor_script_contents();
            fs::write(&path, script.as_bytes())?;
            #[cfg(unix)]
            make_executable(&path)?;
            msg_editor_path = Some(path);
        }

        Ok(Self {
            _dir: dir,
            seq_editor_path,
            todo_path,
            msg_editor_path,
            msgs_dir,
        })
    }

    /// Keep reword data with Git's sequencer state when the initial command
    /// pauses. Git removes the containing `rebase-merge` directory when the
    /// rebase completes, aborts, or quits.
    fn persist_reword_state(&self, git_dir: &Path) -> std::io::Result<()> {
        let Some(editor) = &self.msg_editor_path else {
            return Ok(());
        };

        let rebase_dir = git_dir.join("rebase-merge");
        if !rebase_dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "interactive rebase state directory is missing",
            ));
        }
        // Stage into a sibling directory and rename into place: the state
        // dir either exists complete or not at all, so an interrupted
        // persist can never be classified Ready with message files missing
        // (continue would then silently keep original messages).
        let state_dir = rebase_dir.join(PERSISTED_REWORD_DIR);
        let staging = rebase_dir.join(PERSISTED_REWORD_STAGING_DIR);
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        let messages_dir = staging.join("messages");
        fs::create_dir_all(&messages_dir)?;

        let persisted_editor = staging.join(MSG_EDITOR_NAME);
        fs::copy(editor, &persisted_editor)?;
        #[cfg(unix)]
        make_executable(&persisted_editor)?;

        // The planned todo travels with the messages so the continue path
        // can tell planned message-editing steps from ones added to the
        // todo outside GitComet.
        fs::copy(&self.todo_path, staging.join(PERSISTED_PLAN_NAME))?;

        for entry in fs::read_dir(&self.msgs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                fs::copy(entry.path(), messages_dir.join(entry.file_name()))?;
            }
        }

        if state_dir.exists() {
            fs::remove_dir_all(&state_dir)?;
        }
        fs::rename(&staging, &state_dir)?;
        Ok(())
    }
}

/// Quotes a script path for `GIT_EDITOR` / `GIT_SEQUENCE_EDITOR`.
///
/// Git treats the value as a shell command run by `sh` — on Windows too, via
/// Git for Windows' bundled shell — so an unquoted path containing spaces
/// word-splits, and POSIX single-quote quoting applies on every platform.
fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

enum PersistedReword {
    /// No GitComet reword plan exists for this rebase.
    Absent,
    Ready {
        editor: PathBuf,
        messages: PathBuf,
        plan: PathBuf,
    },
    /// A plan was persisted but is missing pieces (external deletion; the
    /// persist itself is atomic): continuing with it would silently drop
    /// rewords.
    Damaged,
}

fn persisted_reword_state(git_dir: &Path) -> PersistedReword {
    let state_dir = git_dir.join("rebase-merge").join(PERSISTED_REWORD_DIR);
    if !state_dir.exists() {
        return PersistedReword::Absent;
    }
    let editor = state_dir.join(MSG_EDITOR_NAME);
    let messages = state_dir.join("messages");
    let plan = state_dir.join(PERSISTED_PLAN_NAME);
    if editor.is_file() && messages.is_dir() && plan.is_file() {
        PersistedReword::Ready {
            editor,
            messages,
            plan,
        }
    } else {
        PersistedReword::Damaged
    }
}

/// Whether the current todo contains a message-editing step the persisted
/// plan never had — a todo edited outside GitComet (e.g. `git rebase
/// --edit-todo` turning a later pick into a reword). The persisted editor
/// would find no message file for it and exit 0, silently keeping the old
/// message instead of the editor session the user asked for. Ids are
/// compared prefix-tolerantly: git may abbreviate ids when the todo is
/// edited or regenerated.
fn rebase_unplanned_message_edit(git_dir: &Path, plan: &Path) -> bool {
    use gitcomet_core::squash::{TodoLineRole, todo_line_commit_word, todo_line_role};
    let editing = |line: &str| {
        matches!(
            todo_line_role(line),
            TodoLineRole::EditsOwnMessage | TodoLineRole::FoldEditsMessage
        )
    };
    let Ok(plan_content) = fs::read_to_string(plan) else {
        // Unreadable plan: refuse rather than risk a silent accept.
        return true;
    };
    let planned: Vec<&str> = plan_content
        .lines()
        .filter(|l| editing(l))
        .filter_map(todo_line_commit_word)
        .collect();
    let Ok(todo) = fs::read_to_string(git_dir.join("rebase-merge").join("git-rebase-todo")) else {
        return false;
    };
    todo.lines()
        .filter(|l| editing(l))
        .any(|line| match todo_line_commit_word(line) {
            Some(id) => !planned
                .iter()
                .any(|p| p.starts_with(id) || id.starts_with(p)),
            None => true,
        })
}

/// Whether continuing the paused interactive rebase can open a commit-message
/// editor: a message-editing step remains in the todo, the paused step itself
/// is a reword, or the paused fold run still has its message editor pending
/// (git opens the run's editor at its last squash/fixup step). Line
/// classification lives in core ([`gitcomet_core::squash::todo_line_role`])
/// so these rules cannot drift from the planner's.
fn rebase_pending_message_edit(git_dir: &Path) -> bool {
    use gitcomet_core::squash::{TodoLineRole, todo_line_role};
    let rebase_dir = git_dir.join("rebase-merge");

    if let Ok(todo) = fs::read_to_string(rebase_dir.join("git-rebase-todo"))
        && todo.lines().any(|line| {
            matches!(
                todo_line_role(line),
                TodoLineRole::EditsOwnMessage | TodoLineRole::FoldEditsMessage
            )
        })
    {
        return true;
    }

    if let Ok(done) = fs::read_to_string(rebase_dir.join("done")) {
        for (steps_back, line) in done.lines().rev().enumerate() {
            match todo_line_role(line) {
                // The paused step: a conflicted reword (or merge) commits and
                // opens the editor on continue. Deeper ones already ran
                // theirs and end the walk like any other commit-creating step.
                TodoLineRole::EditsOwnMessage if steps_back == 0 => return true,
                TodoLineRole::EditsOwnMessage => break,
                // A squash or fixup -c/-C anywhere in the paused fold run
                // means the run's message editor has not run yet.
                TodoLineRole::FoldEditsMessage => return true,
                // Plain fixups extend the run; drops are transparent to it.
                TodoLineRole::FoldSilent | TodoLineRole::Transparent => {}
                TodoLineRole::Other => break,
            }
        }
    }
    false
}

/// Parses `git log -z --format=%H%x00%s%x00%B` output: a flat sequence of
/// NUL-separated field triples. Every record must have exactly three fields
/// and a full hex commit id — the ids are later written into the rebase todo,
/// so malformed output must fail here rather than corrupt the todo.
fn parse_interactive_rebase_log(output: &str) -> Result<Vec<InteractiveRebaseEntry>> {
    let mut fields: Vec<&str> = output.split('\0').collect();
    // `-z` terminates the final record, leaving one empty trailing field.
    if fields.last() == Some(&"") {
        fields.pop();
    }
    if !fields.len().is_multiple_of(3) {
        return Err(Error::new(ErrorKind::Backend(
            "unexpected git log output while preparing interactive rebase".to_string(),
        )));
    }

    fields
        .chunks_exact(3)
        .map(|record| {
            let (sha, summary, message) = (record[0], record[1], record[2]);
            let full_hex_id =
                (sha.len() == 40 || sha.len() == 64) && sha.bytes().all(|b| b.is_ascii_hexdigit());
            if !full_hex_id {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "unexpected commit id {sha:?} in git log output while preparing \
                     interactive rebase"
                ))));
            }
            Ok(InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: sha.to_string(),
                summary: summary.to_string(),
                message: message.trim_end_matches('\n').to_string(),
                new_message: None,
            })
        })
        .collect()
}

fn build_todo_content(entries: &[InteractiveRebaseEntry]) -> String {
    entries
        .iter()
        .map(|e| {
            let safe_summary = e.summary.replace('\n', " ");
            format!(
                "{} {} {}\n",
                e.action.to_todo_str(),
                e.commit_id,
                safe_summary
            )
        })
        .collect()
}

#[cfg(unix)]
fn make_executable(path: &PathBuf) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
}

// The Windows variants below mirror the unix ones, but first normalize the
// Windows-style paths handed over in the environment: a `\` is an escape in
// sh glob patterns and `dirname` only splits on `/`. (Cygwin file operations
// accept forward-slash `C:/...` paths natively.)

#[cfg(windows)]
fn seq_editor_script_contents() -> &'static [u8] {
    br#"#!/bin/sh
cp "$(printf '%s' "$GITCOMET_TODO_FILE" | tr '\\' /)" "$1"
"#
}

#[cfg(not(windows))]
fn seq_editor_script_contents() -> &'static [u8] {
    b"#!/bin/sh\ncp \"$GITCOMET_TODO_FILE\" \"$1\"\n"
}

#[cfg(windows)]
fn msg_editor_script_contents() -> String {
    r#"#!/bin/sh
winpath() { printf '%s' "$1" | tr '\\' /; }
sha=
message_file=$(winpath "$1")
if [ -n "$GITCOMET_GIT_DIR" ]; then
  git_dir=$(winpath "$GITCOMET_GIT_DIR")
else
  git_dir=$(dirname "$message_file")
fi
msgs_dir=$(winpath "$GITCOMET_MSGS_DIR")
if [ -f "$git_dir/REBASE_HEAD" ]; then
  sha=$(tr -d '\r' <"$git_dir/REBASE_HEAD")
elif [ -f "$git_dir/rebase-merge/done" ]; then
  sha=$(tail -n 1 "$git_dir/rebase-merge/done" | tr -d '\r' | cut -d' ' -f2)
fi
# Message files are keyed by full object id; reject non-hex tokens (a
# non-pick done line) and match by prefix so an abbreviated id in the
# done file still finds its file.
case "$sha" in
  '' | *[!0-9a-f]*) exit 0 ;;
esac
for msg_file in "$msgs_dir/$sha"*; do
  if [ -f "$msg_file" ]; then
    cp "$msg_file" "$message_file" || exit 1
  fi
  break
done
exit 0
"#
    .to_string()
}

#[cfg(not(windows))]
fn msg_editor_script_contents() -> String {
    r#"#!/bin/sh
sha=
message_file=$1
git_dir=${GITCOMET_GIT_DIR:-$(dirname "$message_file")}
if [ -f "$git_dir/REBASE_HEAD" ]; then
  sha=$(cat "$git_dir/REBASE_HEAD")
elif [ -f "$git_dir/rebase-merge/done" ]; then
  sha=$(tail -n 1 "$git_dir/rebase-merge/done" | cut -d' ' -f2)
fi
# Message files are keyed by full object id; reject non-hex tokens (a
# non-pick done line) and match by prefix so an abbreviated id in the
# done file still finds its file.
case "$sha" in
  '' | *[!0-9a-f]*) exit 0 ;;
esac
for msg_file in "$GITCOMET_MSGS_DIR/$sha"*; do
  if [ -f "$msg_file" ]; then
    cp "$msg_file" "$message_file" || exit 1
  fi
  break
done
exit 0
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_interactive_rebase_log, shell_quote_path};
    use std::path::Path;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn editor_path_uses_posix_shell_quotes() {
        assert_eq!(
            shell_quote_path(Path::new("/repo with 'quotes'/editor.sh")),
            r"'/repo with '\''quotes'\''/editor.sh'"
        );
        assert_eq!(
            shell_quote_path(Path::new(r"C:\repo with spaces\editor.sh")),
            r"'C:\repo with spaces\editor.sh'"
        );
    }

    #[test]
    fn parses_records_with_multiline_bodies_and_control_bytes() {
        let output = format!(
            "{SHA_A}\0Subject A\0Subject A\n\nBody \x1e with \x1f separators\n\0\
             {SHA_B}\0Subject B\0Subject B\n\0"
        );
        let entries = parse_interactive_rebase_log(&output).expect("parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].commit_id, SHA_A);
        assert_eq!(entries[0].summary, "Subject A");
        assert_eq!(
            entries[0].message,
            "Subject A\n\nBody \x1e with \x1f separators"
        );
        assert_eq!(entries[1].commit_id, SHA_B);
        assert_eq!(entries[1].message, "Subject B");
    }

    #[test]
    fn empty_output_parses_to_no_entries() {
        assert_eq!(parse_interactive_rebase_log("").expect("parse").len(), 0);
    }

    #[test]
    fn rejects_record_with_missing_fields() {
        let output = format!("{SHA_A}\0Subject only\0");
        assert!(parse_interactive_rebase_log(&output).is_err());
    }

    #[test]
    fn rejects_record_with_invalid_commit_id() {
        let output = "not-a-sha\0Subject\0Subject\n\0";
        assert!(parse_interactive_rebase_log(output).is_err());
    }
}
