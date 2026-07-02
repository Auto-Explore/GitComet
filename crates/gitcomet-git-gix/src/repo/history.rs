use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, run_git_capture, run_git_with_output, validate_hex_commit_id,
    validate_ref_like_arg,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, ResetMode, Result};

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
        let head = gix_head_id_or_none(&repo)?.ok_or_else(|| {
            Error::new(ErrorKind::Backend("squash: HEAD is unborn".to_string()))
        })?;
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

        let (author_name, author_email, author_date) =
            commit_author_env(&repo, oldest.as_ref())?;

        // The squash commit reuses HEAD's tree with the range's base as its
        // parent, so the worktree and index are never touched.
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("commit-tree")
            .arg(format!("{}^{{tree}}", expected_head.as_ref()))
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
        cmd.arg("rebase").arg("--continue");
        run_git_with_output(cmd, "git rebase --continue")
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
