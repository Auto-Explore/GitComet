use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_capture, run_git_raw_output,
    run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, InteractiveRebaseAction, InteractiveRebaseEntry, ResetMode, Result,
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

#[cfg(windows)]
const MSG_EDITOR_NAME: &str = "gitcomet-msg-editor.cmd";
#[cfg(not(windows))]
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
        self.run_rebase_step_output(cmd, "git rebase --continue")
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
            Ok(output) => Ok(output),
            Err(rebase_error) => {
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

    pub(super) fn rebase_in_progress_impl(&self) -> Result<bool> {
        let repo = self._repo.to_thread_local();
        Ok(matches!(
            repo.state(),
            Some(
                gix::state::InProgress::Rebase
                    | gix::state::InProgress::RebaseInteractive
                    | gix::state::InProgress::ApplyMailbox
                    | gix::state::InProgress::ApplyMailboxRebase
            )
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
        cmd.args(["rebase", "-i", "--", base]);

        let label = format!("git rebase -i {base}");
        let mut result = self.run_rebase_step_output(cmd, &label);
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

        #[cfg(windows)]
        let seq_editor_name = "gitcomet-seq-editor.cmd";
        #[cfg(not(windows))]
        let seq_editor_name = "gitcomet-seq-editor.sh";
        let seq_editor_path = dir.path().join(seq_editor_name);
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
/// Git treats the value as a shell command, so an unquoted path containing
/// spaces word-splits. On Windows the script is a batch file and eventually
/// crosses into `cmd.exe`, where single quotes are literal; use double quotes
/// there and escape the characters that Git's intermediate `sh` expands.
#[cfg(windows)]
fn shell_quote_path(path: &Path) -> String {
    windows_shell_quote_path(&path.to_string_lossy())
}

#[cfg(any(windows, test))]
fn windows_shell_quote_path(path: &str) -> String {
    let path = path.replace('"', r#"\""#);
    let path = path.replace('$', r"\$").replace('`', r"\`");
    format!(r#""{path}""#)
}

#[cfg(not(windows))]
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

#[cfg(windows)]
fn seq_editor_script_contents() -> &'static [u8] {
    b"@echo off\ncopy /Y \"%GITCOMET_TODO_FILE%\" \"%~1\" >nul\n"
}

#[cfg(not(windows))]
fn seq_editor_script_contents() -> &'static [u8] {
    b"#!/bin/sh\ncp \"$GITCOMET_TODO_FILE\" \"$1\"\n"
}

#[cfg(windows)]
fn msg_editor_script_contents() -> String {
    r#"@echo off
set "EDITOR_GIT_DIR=%GITCOMET_GIT_DIR%"
if "%EDITOR_GIT_DIR%"=="" set "EDITOR_GIT_DIR=%~dp1."
set "REBASE_HEAD_FILE=%EDITOR_GIT_DIR%\REBASE_HEAD"
set "DONE_FILE=%EDITOR_GIT_DIR%\rebase-merge\done"
set "sha="
if exist "%REBASE_HEAD_FILE%" set /p sha=<"%REBASE_HEAD_FILE%"
if "%sha%"=="" if exist "%DONE_FILE%" (
  for /f "tokens=2" %%s in ('type "%DONE_FILE%"') do set "sha=%%s"
)
if "%sha%"=="" exit /b 0
rem Message files are keyed by full object id; match by prefix so an
rem abbreviated id in the done file still finds its file.
for %%f in ("%GITCOMET_MSGS_DIR%\%sha%*") do (
  copy /Y "%%f" "%~1" >nul
  exit /b 0
)
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
    #[cfg(not(windows))]
    use super::shell_quote_path;
    use super::{parse_interactive_rebase_log, windows_shell_quote_path};
    #[cfg(not(windows))]
    use std::path::Path;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn editor_path_uses_cmd_compatible_quotes_on_windows() {
        assert_eq!(
            windows_shell_quote_path(r"C:\repo with spaces\editor.cmd"),
            r#""C:\repo with spaces\editor.cmd""#
        );
        assert_eq!(
            windows_shell_quote_path(r"C:\repo$`\editor.cmd"),
            r#""C:\repo\$\`\editor.cmd""#
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn editor_path_uses_posix_shell_quotes() {
        assert_eq!(
            shell_quote_path(Path::new("/repo with 'quotes'/editor.sh")),
            r"'/repo with '\''quotes'\''/editor.sh'"
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
