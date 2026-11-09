use super::GixRepo;
use super::history::{gix_head_id_or_none, peel_commit};
use crate::util::{
    git_command_failed_error, run_git_capture, run_git_raw_output, run_git_with_output,
    validate_hex_commit_id, validate_ref_like_arg, with_shared_git_auth,
};
use gitcomet_core::domain::{CommitId, short_commit_id};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{
    CommandOutput, InteractiveRebaseAction, InteractiveRebaseEntry, Result, RewordCommitOutcome,
};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const PERSISTED_REWORD_DIR: &str = "gitcomet-reword";
const PERSISTED_REWORD_STAGING_DIR: &str = "gitcomet-reword.staging";
const PERSISTED_PLAN_NAME: &str = "planned-todo";
const PERSISTED_REWORD_OWNER: &str = "commit-message-owner.json";

// A POSIX sh script on every platform: Git for Windows also runs editors
// through its bundled `sh`, and a shebang script survives a script path
// containing spaces — a batch file does not, because the MSYS runtime
// re-spawns it via `cmd.exe /c` with the path unquoted.
const MSG_EDITOR_NAME: &str = "gitcomet-msg-editor.sh";

fn reword_error(message: &str) -> Error {
    Error::new(ErrorKind::Backend(format!("reword: {message}")))
}

/// Keep the original failure classification and captured output, including
/// authentication, while making recovery failures visible in the command log.
fn reword_error_with_diagnostic(error: Error, diagnostic: &str) -> Error {
    match error.kind() {
        ErrorKind::Git(failure) => {
            let detail = format!(
                "{}\n\n{diagnostic}",
                failure.detail().unwrap_or("Git command failed")
            );
            let mut stderr = failure.stderr().to_vec();
            stderr.extend_from_slice(format!("\n{diagnostic}\n").as_bytes());
            let mut combined = GitFailure::new(
                failure.command(),
                failure.id(),
                failure.exit_code(),
                failure.stdout().to_vec(),
                stderr,
                Some(detail),
            );
            if let Some(outcome) = failure.auth_outcome() {
                combined = combined.with_auth_outcome(outcome.clone());
            }
            Error::new(ErrorKind::Git(combined)).with_source(error)
        }
        _ => reword_error(&format!("{error}\n{diagnostic}")).with_source(error),
    }
}

/// Kept inside Git's own rebase state, so aborting or completing the rebase
/// invalidates ownership automatically. Generic interactive rebases do not
/// have this record, even if they happen to reword the same commit.
struct RewordCommitOwner {
    original_head: String,
    target: String,
    parent: String,
    message: String,
    distance: usize,
}

impl RewordCommitOwner {
    fn matches_rebase(&self, git_dir: &Path) -> bool {
        let state = git_dir.join("rebase-merge");
        [("orig-head", &self.original_head), ("onto", &self.parent)]
            .iter()
            .all(|(name, expected)| {
                fs::read_to_string(state.join(name))
                    .is_ok_and(|actual| actual.trim() == expected.as_str())
            })
    }

    fn persist(&self, git_dir: &Path) -> Result<()> {
        let PersistedReword::Ready { messages, .. } = persisted_reword_state(git_dir) else {
            return Err(reword_error(
                "could not preserve the paused reword's message plan",
            ));
        };
        if !self.matches_rebase(git_dir)
            || fs::read(messages.join(&self.target)).ok().as_deref()
                != Some(self.message.as_bytes())
        {
            return Err(reword_error(
                "the paused rebase no longer matches the message edit",
            ));
        }
        let record = serde_json::json!({
            "original_head": self.original_head,
            "target": self.target,
            "parent": self.parent,
            "message": self.message,
            "distance": self.distance,
        });
        fs::write(
            git_dir.join("rebase-merge").join(PERSISTED_REWORD_OWNER),
            record.to_string(),
        )
        .map_err(|error| {
            Error::from(error).with_context("preserve the paused commit message edit", None)
        })
    }

    fn load(git_dir: &Path, target: &CommitId, message: &str) -> Option<Self> {
        let bytes = fs::read(git_dir.join("rebase-merge").join(PERSISTED_REWORD_OWNER)).ok()?;
        let record: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let owner = Self {
            original_head: record.get("original_head")?.as_str()?.into(),
            target: record.get("target")?.as_str()?.into(),
            parent: record.get("parent")?.as_str()?.into(),
            message: record.get("message")?.as_str()?.into(),
            distance: record.get("distance")?.as_u64()?.try_into().ok()?,
        };
        let PersistedReword::Ready { messages, .. } = persisted_reword_state(git_dir) else {
            return None;
        };
        (owner.target.eq_ignore_ascii_case(target.as_ref())
            && owner.message == message
            && owner.matches_rebase(git_dir)
            && fs::read(messages.join(&owner.target)).ok().as_deref() == Some(message.as_bytes()))
        .then_some(owner)
    }
}

/// Immutable commit content captured before rewriting. All lookup failures
/// after a successful Git command are optional selection metadata failures.
struct RewordCommitIdentity {
    original: gix::ObjectId,
    parents: Vec<gix::ObjectId>,
    tree: gix::ObjectId,
}

impl RewordCommitIdentity {
    fn from_commit(commit: &gix::Commit<'_>) -> Result<Self> {
        Ok(Self {
            original: commit.id,
            parents: commit.parent_ids().map(|id| id.detach()).collect(),
            tree: commit
                .tree_id()
                .map_err(|error| reword_error(&format!("cannot read the commit's tree: {error}")))?
                .detach(),
        })
    }

    fn matches(&self, commit: &gix::Commit<'_>) -> bool {
        commit.id != self.original
            && commit
                .parent_ids()
                .map(|id| id.detach())
                .eq(self.parents.iter().copied())
            && commit.tree_id().is_ok_and(|id| id.detach() == self.tree)
    }
}

/// Compare messages using Git's whitespace cleanup: trim line endings,
/// collapse blank runs, and drop surrounding blank lines. Do not strip
/// comments: the reword editors preserve intentional comment-prefixed lines.
/// Applying this to both sides also supports `commit.cleanup=verbatim`.
fn normalized_reword_message(message: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(message.len());
    let mut blank = false;
    for mut line in message.split(|byte| *byte == b'\n') {
        while line
            .last()
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            line = &line[..line.len() - 1];
        }
        if line.is_empty() {
            blank = !normalized.is_empty();
            continue;
        }
        if blank {
            normalized.push(b'\n');
        }
        normalized.extend_from_slice(line);
        normalized.push(b'\n');
        blank = false;
    }
    normalized
}

/// Whether a failed reword rebase stopped for credentials the auth prompt
/// will supply before continuing it: the failures the state layer raises a
/// prompt for, judged by the askpass outcome every git command reports.
fn reword_stop_resumes_after_auth(error: &Error) -> bool {
    use gitcomet_core::auth::AuthOutcome;
    let ErrorKind::Git(failure) = error.kind() else {
        return false;
    };
    !matches!(
        failure.id(),
        GitFailureId::WorktreeWouldBeOverwritten
            | GitFailureId::StoppedAtConflicts
            | GitFailureId::Timeout
    ) && matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_) | AuthOutcome::Rejected(_))
    )
}

/// The number of first-parent steps from `head` down to `target`, and
/// `target`'s parent. Rewording replays that range with a flattening
/// `git rebase -i`, so every commit on it must have exactly one parent,
/// `target` included.
fn reword_first_parent_distance(
    repo: &gix::Repository,
    head: gix::ObjectId,
    target: gix::ObjectId,
    short: &str,
) -> Result<(usize, gix::ObjectId)> {
    let mut current = head;
    let mut distance = 0;
    loop {
        let commit = peel_commit(repo, &current.to_string())?;
        let mut parents = commit.parent_ids().map(|id| id.detach());
        let (first, second) = (parents.next(), parents.next());
        if current == target {
            return match (first, second) {
                (Some(parent), None) => Ok((distance, parent)),
                (None, _) => Err(reword_error(&format!(
                    "{short} is the root commit; only HEAD can be reworded there"
                ))),
                (Some(_), Some(_)) => Err(reword_error(&format!(
                    "{short} is a merge commit; rewording it would flatten the merge"
                ))),
            };
        }
        match (first, second) {
            (Some(parent), None) => current = parent,
            (None, _) => {
                return Err(reword_error(&format!(
                    "{short} is not on the current branch's first-parent history"
                )));
            }
            (Some(_), Some(_)) => {
                return Err(reword_error(&format!(
                    "a merge commit lies between {short} and HEAD; rewording it would \
                     flatten the merge"
                )));
            }
        }
        distance += 1;
    }
}

impl GixRepo {
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
        // todo unreplayable). `--no-show-signature` because
        // `log.showSignature=true` would print the signature check into the
        // first field of each record.
        cmd.args([
            "log",
            "--no-show-signature",
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

    pub(super) fn reword_commit_with_output_impl(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        with_shared_git_auth(|| self.reword_commit_with_output_inner(commit_id, message))
    }

    fn reword_commit_with_output_inner(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        validate_hex_commit_id(commit_id)?;
        if message.trim().is_empty() {
            return Err(reword_error("the commit message must not be empty"));
        }

        // The target was chosen from a possibly stale log snapshot; check it
        // against the live repository.
        let repo = self.reopen_repo()?;
        let head = gix_head_id_or_none(&repo)?.ok_or_else(|| reword_error("HEAD is unborn"))?;
        let target_commit = peel_commit(&repo, commit_id.as_ref())?;
        let target = target_commit.id;
        let identity = RewordCommitIdentity::from_commit(&target_commit)?;
        if let Some(operation) = self.operation_in_progress_label()? {
            return Err(reword_error(&format!(
                "{operation} is in progress; finish or abort it first"
            )));
        }
        if target == head {
            return self.reword_head_commit(message, &identity);
        }

        let short = short_commit_id(commit_id.as_ref());
        let target_hex = target.to_string();
        let label = "git merge-base --is-ancestor";
        let mut cmd = self.git_workdir_cmd();
        cmd.args([
            "merge-base",
            "--is-ancestor",
            &target_hex,
            &head.to_string(),
        ]);
        let output = run_git_raw_output(cmd, label)?;
        match output.status.code() {
            Some(0) => {}
            Some(1) => {
                return Err(reword_error(&format!(
                    "{short} is not part of the current branch"
                )));
            }
            _ => return Err(git_command_failed_error(label, output)),
        }
        let (distance, parent) = reword_first_parent_distance(&repo, head, target, short)?;

        // The rebase replays the whole listed range, so the plan must cover
        // it exactly: a pick for every descendant, a reword for the target.
        let base = parent.to_string();
        let mut entries = self.list_commits_for_interactive_rebase_impl(&base)?;
        if entries.len() != distance + 1 {
            return Err(reword_error(&format!(
                "the commits after {short} do not form a linear history"
            )));
        }
        for entry in &mut entries {
            if entry.commit_id.eq_ignore_ascii_case(&target_hex) {
                entry.action = InteractiveRebaseAction::Reword;
                entry.new_message = Some(message.to_string());
            } else {
                entry.action = InteractiveRebaseAction::Pick;
            }
        }
        if !entries
            .iter()
            .any(|entry| entry.action == InteractiveRebaseAction::Reword)
        {
            return Err(reword_error(&format!(
                "{short} is not in the commits after its parent"
            )));
        }

        let label = format!("git rebase -i {} (reword {short})", short_commit_id(&base));
        let owner = RewordCommitOwner {
            original_head: head.to_string(),
            target: target_hex,
            parent: base.clone(),
            message: message.to_string(),
            distance,
        };
        let result = self.run_planned_rebase(&entries, &base, &label);
        self.finish_reword_rebase(result, &owner, &identity)
    }

    pub(super) fn reword_commit_in_progress_impl(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<bool> {
        validate_hex_commit_id(commit_id)?;
        Ok(RewordCommitOwner::load(self.repo().path(), commit_id, message).is_some())
    }

    pub(super) fn continue_reword_commit_with_output_impl(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        with_shared_git_auth(|| self.continue_reword_commit_with_output_inner(commit_id, message))
    }

    fn continue_reword_commit_with_output_inner(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        validate_hex_commit_id(commit_id)?;
        let owner =
            RewordCommitOwner::load(self.repo().path(), commit_id, message).ok_or_else(|| {
                reword_error("the paused rebase does not belong to this message edit")
            })?;
        let repo = self.reopen_repo()?;
        let target = peel_commit(&repo, commit_id.as_ref())?;
        let identity = RewordCommitIdentity::from_commit(&target)?;
        // Edits made while a signing prompt is open are not part of this
        // operation, and must not be discarded by a failed retry's rollback.
        let mut check = self.git_workdir_cmd();
        check.args(["diff", "--quiet", "--ignore-submodules=none"]);
        let label = "git diff --quiet --ignore-submodules=none";
        let output = run_git_raw_output(check, label)?;
        match output.status.code() {
            Some(0) => {}
            Some(1) => {
                return Err(reword_error(
                    "stash your worktree changes before retrying this message edit",
                ));
            }
            _ => return Err(git_command_failed_error(label, output)),
        }
        let amended = if gix_head_id_or_none(&repo)? == Some(target.id) {
            // A failed reword amend has no staged diff. Git's --continue
            // skips it, leaving the original message in place, unless we
            // retry the amend Git explicitly asks the user to perform.
            let state = repo.path().join("rebase-merge");
            let stopped_at_target = ["amend", "stopped-sha"].iter().all(|name| {
                fs::read_to_string(state.join(name)).is_ok_and(|id| id.trim() == owner.target)
            });
            let is_reword_step = fs::read_to_string(state.join("done")).is_ok_and(|done| {
                done.lines().next_back().is_some_and(|line| {
                    let mut words = line.split_whitespace();
                    words
                        .next()
                        .and_then(InteractiveRebaseAction::from_todo_word)
                        == Some(InteractiveRebaseAction::Reword)
                        && words.next().is_some_and(|id| owner.target.starts_with(id))
                })
            });
            if !stopped_at_target || !is_reword_step {
                return Err(reword_error(
                    "the paused rebase no longer has this message edit pending",
                ));
            }
            if !self.index_matches_head()? {
                return Err(reword_error(
                    "unstage your changes before retrying this message edit",
                ));
            }
            match self.reword_head_commit(message, &identity) {
                Ok(outcome) => Some(outcome.output),
                Err(error) => return self.finish_reword_rebase(Err(error), &owner, &identity),
            }
        } else {
            self.discard_rescheduled_pick_result()?;
            None
        };
        let result =
            self.run_rebase_step_output(self.rebase_continue_command()?, "git rebase --continue");
        let mut outcome = self.finish_reword_rebase(result, &owner, &identity)?;
        if let Some(amended) = amended {
            outcome.output =
                CommandOutput::combine("git rebase --continue", &[amended, outcome.output]);
        }
        Ok(outcome)
    }

    /// Git before 2.56 reschedules a pick whose commit failed (a signing
    /// challenge) but leaves its result staged without `rebase-merge/message`,
    /// so `--continue` refuses. Unstage exactly that result so the pick replays.
    fn discard_rescheduled_pick_result(&self) -> Result<()> {
        let state = self.repo().path().join("rebase-merge");
        if state.join("message").exists() {
            return Ok(());
        }
        let todo = fs::read_to_string(state.join("git-rebase-todo")).unwrap_or_default();
        let mut words = todo.lines().next().unwrap_or_default().split_whitespace();
        let (Some(action), Some(id)) = (words.next(), words.next()) else {
            return Ok(());
        };
        if InteractiveRebaseAction::from_todo_word(action) != Some(InteractiveRebaseAction::Pick)
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Ok(());
        }
        // A reword replays every descendant onto an identical tree, so the
        // staged result is the picked commit's tree; extra staged edits are
        // left for Git to report.
        if !self.index_matches(Some(id))? {
            return Ok(());
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["read-tree", "-m", "-u", "HEAD"]);
        run_git_with_output(cmd, "git read-tree -m -u HEAD").map(|_| ())
    }

    /// A signing stop keeps its owned rebase. Every other stop rolls back,
    /// including one reached by a retry after signing was unlocked.
    fn finish_reword_rebase(
        &self,
        result: Result<CommandOutput>,
        owner: &RewordCommitOwner,
        identity: &RewordCommitIdentity,
    ) -> Result<RewordCommitOutcome> {
        let repo = self.repo();
        let paused = owner.matches_rebase(repo.path());
        let output = match result {
            Ok(output) if !paused && output.exit_code == Some(0) => output,
            Ok(output) => {
                let error = reword_error(&format!(
                    "the replay stopped before finishing\n{}",
                    output.stderr.trim()
                ));
                return Err(if paused {
                    self.rollback_reword_rebase(error)
                } else {
                    error
                });
            }
            Err(error) if paused && reword_stop_resumes_after_auth(&error) => {
                if let Err(persist_error) = owner.persist(repo.path()) {
                    return Err(self.rollback_reword_rebase(reword_error_with_diagnostic(
                        error,
                        &format!("{persist_error}; the message edit cannot be resumed safely"),
                    )));
                }
                return Err(error);
            }
            Err(error) if paused => return Err(self.rollback_reword_rebase(error)),
            Err(error) => return Err(error),
        };
        let rewritten_id = self.reworded_commit_id(owner.distance, identity, &owner.message);
        Ok(RewordCommitOutcome {
            output,
            rewritten_id,
        })
    }

    fn reworded_commit_id(
        &self,
        distance: usize,
        identity: &RewordCommitIdentity,
        message: &str,
    ) -> Option<CommitId> {
        let repo = self.reopen_repo().ok()?;
        let reworded = peel_commit(&repo, &format!("HEAD~{distance}")).ok()?;
        let actual_message = reworded.message_raw().ok()?;
        (identity.matches(&reworded)
            && normalized_reword_message(actual_message.as_ref())
                == normalized_reword_message(message.as_bytes()))
        .then(|| CommitId(reworded.id.to_string().into()))
    }

    fn rollback_reword_rebase(&self, error: Error) -> Error {
        match self.abort_reword_rebase() {
            Ok(()) => error,
            Err(cleanup) => {
                reword_error_with_diagnostic(error, &format!("Rollback also failed: {cleanup}"))
            }
        }
    }

    fn abort_reword_rebase(&self) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--abort");
        run_git_with_output(cmd, "git rebase --abort").map(|_| ())
    }

    /// Amends HEAD's message only. `--only` with no paths leaves whatever is
    /// staged out of the amended commit, so it works on a dirty worktree, and
    /// `--allow-empty` keeps a commit whose tree matches its parent's
    /// rewordable. A merge HEAD keeps its parents. Lines starting with the
    /// comment character survive a `commit.cleanup=strip` here as they do on
    /// the rebase path.
    fn reword_head_commit(
        &self,
        message: &str,
        identity: &RewordCommitIdentity,
    ) -> Result<RewordCommitOutcome> {
        self.refuse_amending_annex_adjustment()?;
        let label = "git commit --amend --only";
        let comment_char = comment_char_override(
            &configured_comment_prefixes(&self.repo()),
            &[message.as_bytes().to_vec()],
        )?;
        let mut cmd = self.git_workdir_cmd();
        if let Some(comment_char) = comment_char {
            cmd.arg("-c").arg(comment_char_config(comment_char));
        }
        cmd.args(["commit", "--amend", "--only", "--allow-empty", "-m"])
            .arg(message);
        let output = run_git_with_output(cmd, label)?;
        let rewritten_id = self.reworded_commit_id(0, identity, message);
        Ok(RewordCommitOutcome {
            output,
            rewritten_id,
        })
    }

    /// Runs `git rebase -i <upstream>` with the planned todo installed via
    /// the sequence editor (and the message editor when the plan rewords or
    /// squashes), keeping reword data alongside git's state if the rebase
    /// pauses so a later continue can service it. `--empty=drop` drops picks
    /// whose changes are already upstream: git would otherwise stop on them,
    /// and the UI exposes no skip control to move past that stop. Commits
    /// that started out empty are unaffected and stay preserved.
    pub(super) fn run_planned_rebase(
        &self,
        entries: &[InteractiveRebaseEntry],
        upstream: &str,
        label: &str,
    ) -> Result<CommandOutput> {
        let scripts = RebaseScripts::create(entries).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "failed to create rebase scripts: {e}"
            )))
            .with_source(e)
            .with_context("create rebase scripts", None)
        })?;

        let mut cmd = self.git_workdir_cmd();
        cmd.env(
            "GIT_SEQUENCE_EDITOR",
            shell_quote_path(&scripts.seq_editor_path),
        );
        if let Some(ref msg_editor) = scripts.msg_editor_path {
            cmd.env("GIT_EDITOR", shell_quote_path(msg_editor));
            cmd.env("GITCOMET_MSGS_DIR", &scripts.msgs_dir);
            let repo = self.repo();
            cmd.env("GITCOMET_GIT_DIR", repo.path());
            if let Some(comment_char) = planned_messages_comment_char(&repo, &scripts.msgs_dir)? {
                cmd.arg("-c").arg(comment_char_config(comment_char));
            }
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
        let repo = self.repo();
        if repo.path().join("rebase-merge").is_dir() {
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
}

/// `core.commentChar` values, in the order git's own `auto` setting tries them.
const COMMENT_CHAR_CANDIDATES: [u8; 10] = *b"#;@!$%^&|:";

/// The comment character a rebase installing `planned` messages must run
/// with, or `None` when no configured prefix starts any of their lines. The
/// message editor hands git each planned message as an edit, and git strips
/// every edited line starting with the prefix (`#123 Fix crash`). Both
/// `core.commentChar` and `core.commentString` count, since the one read last
/// wins; `auto` picks from git's own message, not the planned one, so it
/// always needs an override.
fn comment_char_override(configured: &[Vec<u8>], planned: &[Vec<u8>]) -> Result<Option<u8>> {
    let lines = || {
        planned
            .iter()
            .flat_map(|message| message.split(|byte| *byte == b'\n'))
    };
    let collides = configured.iter().any(|prefix| {
        prefix.eq_ignore_ascii_case(b"auto") || lines().any(|line| line.starts_with(prefix))
    });
    if !collides {
        return Ok(None);
    }
    COMMENT_CHAR_CANDIDATES
        .into_iter()
        .find(|candidate| !lines().any(|line| line.first() == Some(candidate)))
        .map(Some)
        .ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "every comment character {} can choose starts a line of a planned \
                 commit message, so git would delete those lines; reword in a terminal \
                 instead",
                gitcomet_core::identity::current().display_name()
            )))
        })
}

/// Every planned message installed for a rebase (none when `msgs_dir` is absent).
fn read_planned_messages(msgs_dir: &Path) -> Result<Vec<Vec<u8>>> {
    let io_error = |error: std::io::Error| Error::new(ErrorKind::Io(error.kind()));
    let entries = match fs::read_dir(msgs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    entries
        .map(|entry| fs::read(entry?.path()))
        .collect::<std::io::Result<_>>()
        .map_err(io_error)
}

/// [`comment_char_override`] for the planned messages in `msgs_dir`.
pub(super) fn planned_messages_comment_char(
    repo: &gix::Repository,
    msgs_dir: &Path,
) -> Result<Option<u8>> {
    comment_char_override(
        &configured_comment_prefixes(repo),
        &read_planned_messages(msgs_dir)?,
    )
}

/// The comment prefixes git's message cleanup would strip lines by.
fn configured_comment_prefixes(repo: &gix::Repository) -> Vec<Vec<u8>> {
    let config = repo.config_snapshot();
    let mut configured: Vec<Vec<u8>> = ["core.commentChar", "core.commentString"]
        .into_iter()
        .filter_map(|key| config.string(key).map(|value| value.to_vec()))
        .collect();
    if configured.is_empty() {
        configured.push(b"#".to_vec());
    }
    configured
}

pub(super) fn comment_char_config(comment_char: u8) -> String {
    format!("core.commentChar={}", char::from(comment_char))
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
pub(super) fn shell_quote_path(path: &Path) -> String {
    gitcomet_core::text_utils::shell_single_quote(&path.to_string_lossy())
}

pub(super) enum PersistedReword {
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
    Damaged { messages: PathBuf },
}

pub(super) fn persisted_reword_state(git_dir: &Path) -> PersistedReword {
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
        PersistedReword::Damaged { messages }
    }
}

/// Whether the current todo contains a message-editing step the persisted
/// plan never had — a todo edited outside GitComet (e.g. `git rebase
/// --edit-todo` turning a later pick into a reword). The persisted editor
/// would find no message file for it and exit 0, silently keeping the old
/// message instead of the editor session the user asked for. Ids are
/// compared prefix-tolerantly: git may abbreviate ids when the todo is
/// edited or regenerated.
pub(super) fn rebase_unplanned_message_edit(git_dir: &Path, plan: &Path) -> bool {
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
pub(super) fn rebase_pending_message_edit(git_dir: &Path) -> bool {
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
        .as_chunks::<3>()
        .0
        .iter()
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
    let capacity = entries.iter().fold(0usize, |capacity, entry| {
        capacity
            .saturating_add(entry.action.to_todo_str().len())
            .saturating_add(entry.commit_id.len())
            .saturating_add(entry.summary.len())
            .saturating_add(3)
    });
    let mut todo = String::with_capacity(capacity);
    for entry in entries {
        let _ = write!(todo, "{} {} ", entry.action.to_todo_str(), entry.commit_id);
        for ch in entry.summary.chars() {
            todo.push(if ch == '\n' { ' ' } else { ch });
        }
        todo.push('\n');
    }
    todo
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

/// The terminal command that continues this rebase. A rebase started with a
/// comment-character override wrote git's own instruction lines with it, so
/// continuing without it would commit them.
pub(super) fn continue_command(comment_char: Option<u8>) -> String {
    match comment_char {
        Some(comment_char) => format!(
            "git -c {} rebase --continue",
            gitcomet_core::text_utils::shell_single_quote(&comment_char_config(comment_char))
        ),
        None => "git rebase --continue".to_string(),
    }
}

pub(super) fn unplanned_rebase_edit_message(comment_char: Option<u8>) -> String {
    format!(
        "this rebase has pending reword/squash steps that {} did not \
         plan; continue it in a terminal with `{}`, or abort the rebase",
        gitcomet_core::identity::current().display_name(),
        continue_command(comment_char)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        build_todo_content, comment_char_override, continue_command, parse_interactive_rebase_log,
        shell_quote_path,
    };
    use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
    use std::path::Path;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    #[test]
    fn comment_char_override_keeps_the_configured_prefix_unless_a_planned_line_starts_with_it() {
        let bytes = |values: &[&str]| -> Vec<Vec<u8>> {
            values
                .iter()
                .map(|value| value.as_bytes().to_vec())
                .collect()
        };
        let decide = |configured: &[&str], planned: &[&str]| {
            comment_char_override(&bytes(configured), &bytes(planned))
        };
        assert_eq!(decide(&["#"], &["Subject\n\nbody"]).unwrap(), None);
        assert_eq!(decide(&[";"], &["#1 Subject"]).unwrap(), None);
        assert_eq!(
            decide(&["#"], &["#1 Subject\n\n; note"]).unwrap(),
            Some(b'@')
        );
        assert_eq!(decide(&["//"], &["/ one slash"]).unwrap(), None);
        assert_eq!(decide(&[";", "#"], &["#1 Subject"]).unwrap(), Some(b';'));
        assert_eq!(decide(&["Auto"], &["Subject"]).unwrap(), Some(b'#'));
        assert!(decide(&["#"], &["#\n;\n@\n!\n$\n%\n^\n&\n|\n:"]).is_err());
    }

    #[test]
    fn continue_hint_quotes_the_comment_char_override() {
        assert_eq!(continue_command(None), "git rebase --continue");
        assert_eq!(
            continue_command(Some(b';')),
            "git -c 'core.commentChar=;' rebase --continue"
        );
    }

    #[test]
    fn reword_message_comparison_matches_git_stripspace_whitespace() {
        assert_eq!(
            super::normalized_reword_message(b" \t\r\nSubject\x0b\x0c \t\r\n\n \t\n# Body\t\r\n\n"),
            b"Subject\x0b\x0c\n\n# Body\n"
        );
        assert_ne!(
            super::normalized_reword_message(b"Subject\nBody"),
            super::normalized_reword_message(b"Subject\nDifferent body")
        );
    }

    #[test]
    fn editor_path_uses_posix_shell_quotes() {
        assert_eq!(
            shell_quote_path(Path::new("/repo with 'quotes'/editor.sh")),
            "'/repo with '\"'\"'quotes'\"'\"'/editor.sh'"
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
    fn todo_content_flattens_every_summary_line_without_temporary_strings() {
        let entries = vec![
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: SHA_A.to_string(),
                summary: "Subject\nbody\n\nlast".to_string(),
                message: String::new(),
                new_message: None,
            },
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Reword,
                commit_id: SHA_B.to_string(),
                summary: "Second subject".to_string(),
                message: String::new(),
                new_message: Some("Replacement".to_string()),
            },
        ];

        assert_eq!(
            build_todo_content(&entries),
            format!("pick {SHA_A} Subject body  last\nreword {SHA_B} Second subject\n")
        );
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
