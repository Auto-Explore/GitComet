use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_capture, run_git_raw_output,
    run_git_simple, run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Remote, RemoteBranch, Upstream};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CancellationToken, CommandOutput, ForcePushLease, PullMode, RemoteUrlKind, Result,
    SafePushAfterCommitContext, SafePushAfterCommitDecision, SafePushAfterCommitTarget,
};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashSet as HashSet;
use std::process::Command;
use std::str;

fn parse_refname_set(output: &str) -> HashSet<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn branches_to_prune(
    branches_output: &str,
    merged: &HashSet<String>,
    existing_tracking_refs: &HashSet<String>,
    current_branch: Option<&str>,
) -> Vec<String> {
    let mut candidates = Vec::new();

    for line in branches_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (branch, upstream) = line.split_once('\t').unwrap_or((line, ""));
        if branch.is_empty() || upstream.is_empty() {
            continue;
        }
        if current_branch == Some(branch) {
            continue;
        }
        if !merged.contains(branch) {
            continue;
        }

        let tracking_ref = format!("refs/remotes/{upstream}");
        if existing_tracking_refs.contains(&tracking_ref) {
            continue;
        }
        candidates.push(branch.to_string());
    }

    candidates
}

fn parse_short_remote_branch_name(short_name: &str) -> Option<(&str, &str)> {
    let short_name = short_name.trim();
    if short_name.is_empty() || short_name.ends_with("/HEAD") {
        return None;
    }
    let (remote, name) = short_name.split_once('/')?;
    if remote.is_empty() || name.is_empty() {
        return None;
    }
    Some((remote, name))
}

fn normalize_remote_url(url: &str) -> String {
    let Some(path) = url.strip_prefix("file://") else {
        return url.to_string();
    };
    let path_bytes = path.as_bytes();
    if path.starts_with('/')
        || path_bytes.len() < 3
        || !path_bytes[0].is_ascii_alphabetic()
        || path_bytes[1] != b':'
        || !matches!(path_bytes[2], b'/' | b'\\')
    {
        return url.to_string();
    }

    // gix serializes Windows drive-letter file remotes as `file://C:/...`.
    let normalized_path = path.replace('\\', "/");
    format!("file:///{normalized_path}")
}

fn safe_push_ref_display(remote: &str, branch: &str) -> String {
    format!("{remote}/{branch}")
}

fn output_mentions_missing_remote_ref(output: &std::process::Output) -> bool {
    let mut text = bytes_to_text_preserving_utf8(&output.stderr);
    text.push('\n');
    text.push_str(&bytes_to_text_preserving_utf8(&output.stdout));
    text.contains("couldn't find remote ref")
        || text.contains("could not find remote ref")
        || text.contains("couldn't find remote branch")
        || text.contains("could not find remote branch")
}

fn run_git_command<S, O>(
    cmd: Command,
    label: &str,
    capture_output: bool,
    run_simple: S,
    run_with_output: O,
) -> Result<CommandOutput>
where
    S: FnOnce(Command, &str) -> Result<()>,
    O: FnOnce(Command, &str) -> Result<CommandOutput>,
{
    if capture_output {
        return run_with_output(cmd, label);
    }

    run_simple(cmd, label)?;
    Ok(CommandOutput::empty_success(label))
}

fn run_git_command_with_optional_output(
    cmd: Command,
    label: &str,
    capture_output: bool,
) -> Result<CommandOutput> {
    run_git_command(
        cmd,
        label,
        capture_output,
        run_git_simple,
        run_git_with_output,
    )
}

impl GixRepo {
    fn best_effort_delete_reference(&self, ref_name: &str) {
        let repo = self._repo.to_thread_local();
        let Ok(Some(reference)) = repo.try_find_reference(ref_name) else {
            return;
        };
        let _ = reference.delete();
    }

    fn preferred_remote_name(&self) -> Result<Option<String>> {
        let remotes = self.list_remotes_impl()?;
        if remotes.is_empty() {
            return Ok(None);
        }
        if remotes.iter().any(|r| r.name == "origin") {
            return Ok(Some("origin".to_string()));
        }
        Ok(Some(remotes[0].name.clone()))
    }

    fn current_branch_name(&self) -> Result<Option<String>> {
        let head = self.current_branch_impl()?;
        let head = head.trim();
        if head.is_empty() || head == "HEAD" {
            return Ok(None);
        }
        Ok(Some(head.to_string()))
    }

    fn branch_upstream(&self, branch_name: &str) -> Result<Option<Upstream>> {
        validate_ref_like_arg(branch_name, "branch name")?;

        let repo = self.reopen_repo()?;
        let ref_name = format!("refs/heads/{branch_name}");
        let Some(reference) = repo
            .try_find_reference(ref_name.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
        else {
            return Ok(None);
        };

        let tracking_ref_name =
            match reference.remote_tracking_ref_name(gix::remote::Direction::Fetch) {
                Some(Ok(name)) => name,
                Some(Err(_)) | None => return Ok(None),
            };

        let upstream_short = tracking_ref_name.shorten().to_str_lossy().into_owned();
        let Some((remote, upstream_branch)) = parse_short_remote_branch_name(&upstream_short)
        else {
            return Ok(None);
        };

        Ok(Some(Upstream {
            remote: remote.to_string(),
            branch: upstream_branch.to_string(),
        }))
    }

    fn branch_has_upstream(&self, branch: &str) -> Result<bool> {
        Ok(self.branch_upstream(branch)?.is_some())
    }

    pub(super) fn list_remotes_impl(&self) -> Result<Vec<Remote>> {
        let repo = self.reopen_repo()?;
        let mut remotes = Vec::new();

        for name in repo.remote_names() {
            let remote = repo.find_remote(&name).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix find_remote {}: {e}",
                    name.to_str_lossy()
                )))
            })?;

            let url = remote
                .url(gix::remote::Direction::Fetch)
                .map(|url| {
                    normalize_remote_url(&bytes_to_text_preserving_utf8(url.to_bstring().as_ref()))
                })
                .filter(|url| !url.is_empty());

            remotes.push(Remote {
                name: name.to_str_lossy().into_owned(),
                url,
            });
        }

        remotes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(remotes)
    }

    pub(super) fn list_remote_branches_impl(&self) -> Result<Vec<RemoteBranch>> {
        self.list_remote_branches_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn list_remote_branches_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let repo = self._repo.to_thread_local();
        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
        let iter = refs
            .remote_branches()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix remote_branches: {e}"))))?;

        let mut branches = Vec::new();
        for reference in iter {
            cancellation.check_cancelled()?;
            let mut reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            let short_name = reference.name().shorten().to_str_lossy().into_owned();
            let Some((remote, name)) = parse_short_remote_branch_name(&short_name) else {
                continue;
            };

            let target = match reference.try_id() {
                Some(id) => id.detach(),
                None => reference
                    .peel_to_id()
                    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel branch: {e}"))))?
                    .detach(),
            };

            branches.push(RemoteBranch {
                remote: remote.to_string(),
                name: name.to_string(),
                target: gitcomet_core::domain::CommitId(target.to_string().into()),
            });
        }

        branches.sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
        cancellation.check_cancelled()?;
        Ok(branches)
    }

    fn fetch_all_with_optional_output_impl(
        &self,
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("fetch").arg("--all");
        if prune {
            cmd.arg("--prune");
        }
        run_git_command_with_optional_output(
            cmd,
            if prune {
                "git fetch --all --prune"
            } else {
                "git fetch --all"
            },
            capture_output,
        )
    }

    pub(super) fn fetch_all_impl(&self, prune: bool) -> Result<()> {
        self.fetch_all_with_optional_output_impl(prune, false)
            .map(|_| ())
    }

    pub(super) fn fetch_all_with_output_impl(&self, prune: bool) -> Result<CommandOutput> {
        self.fetch_all_with_optional_output_impl(prune, true)
    }

    fn pull_with_optional_output_impl(
        &self,
        mode: PullMode,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let branch = self.current_branch_name()?;
        let has_upstream = match branch.as_deref() {
            Some(branch) => self.branch_has_upstream(branch)?,
            None => true,
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("pull");
        match mode {
            // Be explicit about ff behavior so we don't create merge commits when a fast-forward
            // is possible, even if the user's git config disables ff.
            PullMode::Default => {
                cmd.arg("--ff");
            }
            PullMode::Merge => {
                cmd.arg("--no-rebase");
                cmd.arg("--ff");
            }
            PullMode::FastForwardIfPossible => {
                cmd.arg("--ff");
            }
            PullMode::FastForwardOnly => {
                cmd.arg("--ff-only");
            }
            PullMode::Rebase => {
                cmd.arg("--rebase");
            }
        }

        if !has_upstream
            && let Some(branch) = branch
            && let Some(remote) = self.preferred_remote_name()?
        {
            validate_ref_like_arg(&remote, "remote name")?;
            validate_ref_like_arg(&branch, "branch name")?;

            cmd.arg("--").arg(&remote).arg(&branch);
            let output = run_git_command_with_optional_output(
                cmd,
                &format!("git pull {remote} {branch}"),
                capture_output,
            )?;

            let mut set_upstream = self.git_workdir_cmd();
            set_upstream
                .arg("branch")
                .arg("--set-upstream-to")
                .arg(format!("{remote}/{branch}"))
                .arg("--")
                .arg(branch);
            run_git_simple(set_upstream, "git branch --set-upstream-to")?;
            return Ok(output);
        }

        run_git_command_with_optional_output(cmd, "git pull", capture_output)
    }

    pub(super) fn pull_impl(&self, mode: PullMode) -> Result<()> {
        self.pull_with_optional_output_impl(mode, false).map(|_| ())
    }

    pub(super) fn pull_with_output_impl(&self, mode: PullMode) -> Result<CommandOutput> {
        self.pull_with_optional_output_impl(mode, true)
    }

    fn push_set_upstream_with_optional_output_impl(
        &self,
        remote: &str,
        branch: &str,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let command_label = format!("git push --set-upstream {remote} HEAD:refs/heads/{branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--set-upstream")
            .arg("--")
            .arg(remote)
            .arg(format!("HEAD:refs/heads/{branch}"));
        run_git_command_with_optional_output(cmd, &command_label, capture_output)
    }

    fn push_head_to_branch_with_optional_output_impl(
        &self,
        remote: &str,
        branch: &str,
        force_with_lease: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let command_label = if force_with_lease {
            format!("git push --force-with-lease {remote} HEAD:refs/heads/{branch}")
        } else {
            format!("git push {remote} HEAD:refs/heads/{branch}")
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        if force_with_lease {
            cmd.arg("--force-with-lease");
        }
        cmd.arg("--")
            .arg(remote)
            .arg(format!("HEAD:refs/heads/{branch}"));
        run_git_command_with_optional_output(cmd, &command_label, capture_output)
    }

    fn push_head_to_branch_with_oid_lease_with_output_impl(
        &self,
        lease: &ForcePushLease,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(&lease.remote, "remote name")?;
        validate_ref_like_arg(&lease.branch, "branch name")?;
        validate_hex_commit_id(&lease.expected)?;
        validate_ref_like_arg(&lease.local_branch, "local branch name")?;
        validate_hex_commit_id(&lease.local_head)?;

        let current_branch = self.current_branch_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected branch {}, but HEAD is detached",
                lease.local_branch
            )))
        })?;
        if current_branch != lease.local_branch {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected branch {}, but current branch is {}",
                lease.local_branch, current_branch
            ))));
        }

        let current_head = self.head_commit_id_impl()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "stale force-push lease: current HEAD does not point to a commit".to_string(),
            ))
        })?;
        if current_head != lease.local_head {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected HEAD {}, but current HEAD is {}",
                lease.local_head, current_head
            ))));
        }

        let lease_ref = format!("refs/heads/{}", lease.branch);
        let lease_arg = format!("--force-with-lease={}:{}", lease_ref, lease.expected);
        let source_ref = format!("{}:{lease_ref}", lease.local_head);
        let command_label = format!("git push {lease_arg} {} {source_ref}", lease.remote);

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg(&lease_arg)
            .arg("--")
            .arg(&lease.remote)
            .arg(source_ref);
        run_git_with_output(cmd, &command_label)
    }

    pub(super) fn head_commit_id_impl(&self) -> Result<Option<CommitId>> {
        let repo = self.reopen_repo()?;
        gix_head_id_or_none(&repo).map(|id| id.map(|id| CommitId(id.to_string().into())))
    }

    fn validate_push_after_commit_target(&self, target: &SafePushAfterCommitTarget) -> Result<()> {
        validate_ref_like_arg(&target.remote, "remote name")?;
        validate_ref_like_arg(&target.branch, "branch name")?;
        validate_ref_like_arg(&target.local_branch, "local branch name")?;
        validate_hex_commit_id(&target.local_head)?;

        let current_branch = self.current_branch_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected branch {}, but HEAD is detached",
                target.local_branch
            )))
        })?;
        if current_branch != target.local_branch {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected branch {}, but current branch is {}",
                target.local_branch, current_branch
            ))));
        }

        let current_head = self.head_commit_id_impl()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "stale push-after-commit target: current HEAD does not point to a commit"
                    .to_string(),
            ))
        })?;
        if current_head != target.local_head {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected HEAD {}, but current HEAD is {}",
                target.local_head, current_head
            ))));
        }

        Ok(())
    }

    fn push_after_commit_target_with_optional_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
        set_upstream: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        self.validate_push_after_commit_target(target)?;

        let source = if set_upstream {
            format!("refs/heads/{}", target.local_branch)
        } else {
            target.local_head.to_string()
        };
        let refspec = format!("{source}:refs/heads/{}", target.branch);
        let command_label = if set_upstream {
            format!(
                "git push --set-upstream {} {}:refs/heads/{}",
                target.remote, source, target.branch
            )
        } else {
            format!(
                "git push {} {}:refs/heads/{}",
                target.remote, source, target.branch
            )
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        if set_upstream {
            cmd.arg("--set-upstream");
        }
        cmd.arg("--").arg(&target.remote).arg(refspec);
        run_git_command_with_optional_output(cmd, &command_label, capture_output)
    }

    pub(super) fn push_after_commit_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_optional_output_impl(target, false, true)
    }

    pub(super) fn push_after_commit_set_upstream_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_optional_output_impl(target, true, true)
    }

    fn fetch_remote_branch_tip_for_safe_push(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<Option<CommitId>> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let remote_ref = format!("refs/heads/{branch}");
        let label = format!("git fetch --refmap= {remote} {remote_ref}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("fetch")
            .arg("--no-tags")
            .arg("--refmap=")
            .arg("--")
            .arg(remote)
            .arg(&remote_ref);
        let output = run_git_raw_output(cmd, &label)?;
        if !output.status.success() {
            if output_mentions_missing_remote_ref(&output) {
                return Ok(None);
            }
            return Err(git_command_failed_error(&label, output));
        }

        let label = "git rev-parse --verify FETCH_HEAD^{commit}";
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-parse")
            .arg("--verify")
            .arg("FETCH_HEAD^{commit}");
        let output = run_git_raw_output(cmd, label)?;
        if !output.status.success() {
            return Err(git_command_failed_error(label, output));
        }

        let tip = bytes_to_text_preserving_utf8(&output.stdout)
            .trim()
            .to_string();
        let tip = CommitId(tip.into());
        validate_hex_commit_id(&tip)?;
        Ok(Some(tip))
    }

    fn commit_is_ancestor(&self, ancestor: &CommitId, descendant: &CommitId) -> Result<bool> {
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

    fn safe_push_decision_for_target(
        &self,
        context: &SafePushAfterCommitContext,
        local_branch: &str,
        remote: String,
        branch: String,
        has_upstream: bool,
    ) -> Result<SafePushAfterCommitDecision> {
        let display_ref = safe_push_ref_display(&remote, &branch);
        let Some(post_head) = context.post_head.as_ref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No commit was created to push.".to_string(),
                lease: None,
            });
        };
        let target = SafePushAfterCommitTarget {
            remote,
            branch,
            local_branch: local_branch.to_string(),
            local_head: post_head.clone(),
        };

        let Some(remote_tip) =
            self.fetch_remote_branch_tip_for_safe_push(&target.remote, &target.branch)?
        else {
            return if has_upstream {
                Ok(SafePushAfterCommitDecision::Blocked {
                    summary: format!(
                        "The configured upstream branch {display_ref} was not found on the remote."
                    ),
                    lease: None,
                })
            } else {
                Ok(SafePushAfterCommitDecision::PushSetUpstream { target })
            };
        };

        if self.commit_is_ancestor(&remote_tip, post_head)? {
            return if has_upstream {
                Ok(SafePushAfterCommitDecision::Push { target })
            } else {
                Ok(SafePushAfterCommitDecision::PushSetUpstream { target })
            };
        }

        if context.amend && context.pre_head.as_ref() == Some(&remote_tip) {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "The amended commit appears to be published at {display_ref}. Use Force push with lease to update it without overwriting newer remote work."
                ),
                lease: Some(ForcePushLease {
                    remote: target.remote,
                    branch: target.branch,
                    expected: remote_tip,
                    local_branch: local_branch.to_string(),
                    local_head: post_head.clone(),
                }),
            });
        }

        Ok(SafePushAfterCommitDecision::Blocked {
            summary: format!(
                "Remote branch {display_ref} changed while committing. Pull or rebase manually, then push again."
            ),
            lease: None,
        })
    }

    fn validate_safe_push_after_commit_context(
        &self,
        local_branch: &str,
        post_head: &CommitId,
    ) -> Result<Option<SafePushAfterCommitDecision>> {
        validate_ref_like_arg(local_branch, "local branch name")?;
        validate_hex_commit_id(post_head)?;

        let Some(current_branch) = self.current_branch_name()? else {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current branch changed from {local_branch} to detached HEAD after committing. Check out {local_branch} and push manually."
                ),
                lease: None,
            }));
        };
        if current_branch != local_branch {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current branch changed from {local_branch} to {current_branch} after committing. Check out {local_branch} and push manually."
                ),
                lease: None,
            }));
        }

        let Some(current_head) = self.head_commit_id_impl()? else {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current HEAD no longer points to the commit created on {local_branch}. Push manually."
                ),
                lease: None,
            }));
        };
        if &current_head != post_head {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current HEAD changed after committing on {local_branch}. Expected {post_head}, but current HEAD is {current_head}. Push manually."
                ),
                lease: None,
            }));
        }

        Ok(None)
    }

    pub(super) fn safe_push_after_commit_impl(
        &self,
        context: &SafePushAfterCommitContext,
    ) -> Result<SafePushAfterCommitDecision> {
        let Some(post_head) = context.post_head.as_ref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No commit was created to push.".to_string(),
                lease: None,
            });
        };
        let Some(local_branch) = context.local_branch.as_deref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "Push after commit needs a checked-out branch.".to_string(),
                lease: None,
            });
        };

        if let Some(decision) =
            self.validate_safe_push_after_commit_context(local_branch, post_head)?
        {
            return Ok(decision);
        }

        if let Some(upstream) = self.branch_upstream(local_branch)? {
            return self.safe_push_decision_for_target(
                context,
                local_branch,
                upstream.remote,
                upstream.branch,
                true,
            );
        }

        let Some(remote) = self.preferred_remote_name()? else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No git remote is configured for push after commit.".to_string(),
                lease: None,
            });
        };

        self.safe_push_decision_for_target(
            context,
            local_branch,
            remote,
            local_branch.to_string(),
            false,
        )
    }

    fn push_with_optional_output_impl(&self, capture_output: bool) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()? {
            if let Some(upstream) = self.branch_upstream(&branch)? {
                return self.push_head_to_branch_with_optional_output_impl(
                    &upstream.remote,
                    &upstream.branch,
                    false,
                    capture_output,
                );
            }

            if let Some(remote) = self.preferred_remote_name()? {
                return self.push_set_upstream_with_optional_output_impl(
                    &remote,
                    &branch,
                    capture_output,
                );
            }
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        run_git_command_with_optional_output(cmd, "git push", capture_output)
    }

    pub(super) fn push_impl(&self) -> Result<()> {
        self.push_with_optional_output_impl(false).map(|_| ())
    }

    pub(super) fn push_with_output_impl(&self) -> Result<CommandOutput> {
        self.push_with_optional_output_impl(true)
    }

    fn push_force_with_optional_output_impl(&self, capture_output: bool) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()?
            && let Some(upstream) = self.branch_upstream(&branch)?
        {
            return self.push_head_to_branch_with_optional_output_impl(
                &upstream.remote,
                &upstream.branch,
                true,
                capture_output,
            );
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push").arg("--force-with-lease");
        run_git_command_with_optional_output(cmd, "git push --force-with-lease", capture_output)
    }

    pub(super) fn push_force_impl(&self) -> Result<()> {
        self.push_force_with_optional_output_impl(false).map(|_| ())
    }

    pub(super) fn push_force_with_output_impl(&self) -> Result<CommandOutput> {
        self.push_force_with_optional_output_impl(true)
    }

    pub(super) fn push_force_with_lease_with_output_impl(
        &self,
        lease: &ForcePushLease,
    ) -> Result<CommandOutput> {
        self.push_head_to_branch_with_oid_lease_with_output_impl(lease)
    }

    pub(super) fn pull_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let command_str = format!("git pull --no-rebase --ff {remote} {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("--no-pager")
            .arg("pull")
            .arg("--no-rebase")
            .arg("--ff")
            .arg("--")
            .arg(remote)
            .arg(branch);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn merge_ref_with_output_impl(&self, reference: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(reference, "reference")?;

        let command_str = format!("git merge --ff --no-edit {reference}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("--no-pager")
            .arg("merge")
            .arg("--ff")
            .arg("--no-edit")
            .arg("--")
            .arg(reference);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn squash_ref_with_output_impl(&self, reference: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(reference, "reference")?;

        let command_str = format!("git merge --squash --no-commit {reference}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("--no-pager")
            .arg("merge")
            .arg("--squash")
            .arg("--no-commit")
            .arg("--")
            .arg(reference);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn add_remote_with_output_impl(
        &self,
        name: &str,
        url: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("add").arg("--").arg(name).arg(url);
        run_git_with_output(cmd, &format!("git remote add {name} {url}"))
    }

    pub(super) fn remove_remote_with_output_impl(&self, name: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("remove").arg("--").arg(name);
        run_git_with_output(cmd, &format!("git remote remove {name}"))
    }

    pub(super) fn set_remote_url_with_output_impl(
        &self,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("set-url");
        match kind {
            RemoteUrlKind::Fetch => {}
            RemoteUrlKind::Push => {
                cmd.arg("--push");
            }
        }
        cmd.arg("--").arg(name).arg(url);
        let label = match kind {
            RemoteUrlKind::Fetch => format!("git remote set-url {name} {url}"),
            RemoteUrlKind::Push => format!("git remote set-url --push {name} {url}"),
        };
        run_git_with_output(cmd, &label)
    }

    pub(super) fn push_set_upstream_impl(&self, remote: &str, branch: &str) -> Result<()> {
        self.push_set_upstream_with_optional_output_impl(remote, branch, false)
            .map(|_| ())
    }

    pub(super) fn push_set_upstream_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        self.push_set_upstream_with_optional_output_impl(remote, branch, true)
    }

    pub(super) fn set_upstream_branch_with_output_impl(
        &self,
        branch: &str,
        upstream: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(branch, "branch name")?;
        let Some((remote, upstream_branch)) = parse_short_remote_branch_name(upstream) else {
            return Err(Error::new(ErrorKind::Backend(
                "invalid upstream: expected remote/branch".to_string(),
            )));
        };
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(upstream_branch, "branch name")?;

        let label = format!("git branch --set-upstream-to {upstream} {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("branch")
            .arg("--set-upstream-to")
            .arg(upstream)
            .arg("--")
            .arg(branch);
        run_git_with_output(cmd, &label)
    }

    pub(super) fn unset_upstream_branch_with_output_impl(
        &self,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(branch, "branch name")?;

        let label = format!("git branch --unset-upstream {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("branch")
            .arg("--unset-upstream")
            .arg("--")
            .arg(branch);
        run_git_with_output(cmd, &label)
    }

    pub(super) fn delete_remote_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let label = format!("git push --delete {remote} {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--delete")
            .arg("--")
            .arg(remote)
            .arg(branch);
        let output = run_git_with_output(cmd, &label)?;

        let refname = format!("refs/remotes/{remote}/{branch}");
        self.best_effort_delete_reference(&refname);

        Ok(output)
    }

    pub(super) fn prune_merged_branches_with_output_impl(&self) -> Result<CommandOutput> {
        let fetch_output = self.fetch_all_with_output_impl(true)?;

        let mut merged_cmd = self.git_workdir_cmd();
        merged_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname:short)")
            .arg("--merged=HEAD")
            .arg("refs/heads");
        let merged_output =
            run_git_capture(merged_cmd, "git for-each-ref --merged=HEAD refs/heads")?;
        let merged = parse_refname_set(&merged_output);

        let mut branches_cmd = self.git_workdir_cmd();
        branches_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname:short)\t%(upstream:short)")
            .arg("refs/heads");
        let branches_output = run_git_capture(
            branches_cmd,
            "git for-each-ref --format=%(refname:short)\\t%(upstream:short) refs/heads",
        )?;

        let mut refs_cmd = self.git_workdir_cmd();
        refs_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname)")
            .arg("refs/remotes");
        let tracking_refs_output = run_git_capture(
            refs_cmd,
            "git for-each-ref --format=%(refname) refs/remotes",
        )?;
        let existing_tracking_refs = parse_refname_set(&tracking_refs_output);

        let current_branch = self.current_branch_name()?;
        let prune_candidates = branches_to_prune(
            &branches_output,
            &merged,
            &existing_tracking_refs,
            current_branch.as_deref(),
        );
        let mut deleted: Vec<String> = Vec::new();
        let mut deleted_outputs: Vec<CommandOutput> = Vec::new();

        for branch in prune_candidates {
            let mut delete_cmd = self.git_workdir_cmd();
            delete_cmd.arg("branch").arg("-d").arg("--").arg(&branch);
            let output = run_git_with_output(delete_cmd, &format!("git branch -d {branch}"))?;
            deleted.push(branch);
            deleted_outputs.push(output);
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        if !fetch_output.stdout.is_empty() {
            stdout.push_str(&fetch_output.stdout);
        }
        if !fetch_output.stderr.is_empty() {
            stderr.push_str(&fetch_output.stderr);
        }
        for output in &deleted_outputs {
            if !output.stdout.is_empty() {
                stdout.push_str(&output.stdout);
            }
            if !output.stderr.is_empty() {
                stderr.push_str(&output.stderr);
            }
        }
        if deleted.is_empty() {
            if !stdout.ends_with('\n') && !stdout.is_empty() {
                stdout.push('\n');
            }
            stdout.push_str("No merged local branches to prune.\n");
        } else {
            if !stdout.ends_with('\n') && !stdout.is_empty() {
                stdout.push('\n');
            }
            stdout.push_str("Pruned merged local branches:\n");
            for branch in deleted {
                stdout.push_str("- ");
                stdout.push_str(&branch);
                stdout.push('\n');
            }
        }

        Ok(CommandOutput {
            command: "git prune merged branches".to_string(),
            stdout,
            stderr,
            exit_code: Some(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        branches_to_prune, normalize_remote_url, parse_refname_set, parse_short_remote_branch_name,
        run_git_command,
    };
    use gitcomet_core::services::CommandOutput;
    use rustc_hash::FxHashSet as HashSet;
    use std::{cell::Cell, process::Command};

    #[test]
    fn parse_refname_set_trims_and_deduplicates_lines() {
        let output =
            "refs/remotes/origin/main\n\n refs/remotes/origin/main \nrefs/remotes/upstream/dev\n";
        let refs = parse_refname_set(output);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("refs/remotes/origin/main"));
        assert!(refs.contains("refs/remotes/upstream/dev"));
    }

    #[test]
    fn branches_to_prune_filters_by_merge_state_tracking_and_current_branch() {
        let branches_output = "\
feature/stale\torigin/feature/stale\n\
feature/tracked\torigin/feature/tracked\n\
feature/unmerged\torigin/feature/unmerged\n\
feature/current\torigin/feature/current\n\
feature/no-upstream\t\n";
        let merged: HashSet<String> = ["feature/stale", "feature/tracked", "feature/current"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        let tracking_refs: HashSet<String> = ["refs/remotes/origin/feature/tracked".to_string()]
            .into_iter()
            .collect();

        let prune = branches_to_prune(
            branches_output,
            &merged,
            &tracking_refs,
            Some("feature/current"),
        );
        assert_eq!(prune, vec!["feature/stale".to_string()]);
    }

    #[test]
    fn parse_short_remote_branch_name_skips_head_and_preserves_nested_branch_paths() {
        assert_eq!(
            parse_short_remote_branch_name("origin/main"),
            Some(("origin", "main"))
        );
        assert_eq!(
            parse_short_remote_branch_name("upstream/feature/topic"),
            Some(("upstream", "feature/topic"))
        );
        assert_eq!(parse_short_remote_branch_name("origin/HEAD"), None);
        assert_eq!(parse_short_remote_branch_name(""), None);
    }

    #[test]
    fn normalize_remote_url_preserves_non_drive_letter_urls() {
        assert_eq!(
            normalize_remote_url("https://example.com/repo.git"),
            "https://example.com/repo.git"
        );
        assert_eq!(
            normalize_remote_url("file:///tmp/repo.git"),
            "file:///tmp/repo.git"
        );
        assert_eq!(
            normalize_remote_url("file://server/share/repo.git"),
            "file://server/share/repo.git"
        );
    }

    #[test]
    fn normalize_remote_url_fixes_windows_drive_letter_file_urls() {
        assert_eq!(
            normalize_remote_url("file://C:/Users/example/repo.git"),
            "file:///C:/Users/example/repo.git"
        );
        assert_eq!(
            normalize_remote_url(r"file://D:\Users\example\repo.git"),
            "file:///D:/Users/example/repo.git"
        );
    }

    #[test]
    fn run_git_command_discard_mode_uses_simple_runner_and_returns_empty_success() {
        let simple_called = Cell::new(false);
        let with_output_called = Cell::new(false);

        let output = run_git_command(
            Command::new("git"),
            "git push",
            false,
            |_, label| {
                simple_called.set(true);
                assert_eq!(label, "git push");
                Ok(())
            },
            |_, _| {
                with_output_called.set(true);
                Ok(CommandOutput::empty_success("unexpected"))
            },
        )
        .expect("discard mode should execute the simple runner");

        assert!(simple_called.get());
        assert!(!with_output_called.get());
        assert_eq!(output, CommandOutput::empty_success("git push"));
    }

    #[test]
    fn run_git_command_capture_mode_uses_output_runner() {
        let simple_called = Cell::new(false);
        let with_output_called = Cell::new(false);
        let expected = CommandOutput {
            command: "git push".to_string(),
            stdout: "stdout".to_string(),
            stderr: "stderr".to_string(),
            exit_code: Some(0),
        };

        let output = run_git_command(
            Command::new("git"),
            "git push",
            true,
            |_, _| {
                simple_called.set(true);
                Ok(())
            },
            |_, label| {
                with_output_called.set(true);
                assert_eq!(label, "git push");
                Ok(expected.clone())
            },
        )
        .expect("capture mode should execute the output runner");

        assert!(!simple_called.get());
        assert!(with_output_called.get());
        assert_eq!(output, expected);
    }
}
