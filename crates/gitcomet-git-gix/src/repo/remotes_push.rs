use super::history::gix_head_id_or_none;
use super::remotes::{
    UpstreamBranch, tracking_refs_for_remote_branch, tracking_refs_of_remote,
    write_branch_upstream_config,
};
use super::{GixRepo, oid_to_arc_str};
use crate::util::{
    bytes_to_text_preserving_utf8, command_output, git_command_failed_error, run_git_raw_output,
    run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Upstream};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CancellationToken, CommandOutput, ForcePushLease, GitHooks, Result, SafePushAfterCommitContext,
    SafePushAfterCommitDecision, SafePushAfterCommitTarget, UPSTREAM_CONFIG_WRITE_FAILED,
};
use rustc_hash::FxHashSet;
use std::str;

/// Whether `--no-verify` skips an LFS upload: Git would run a `pre-push`
/// hook, and the pushed commits add pointers LFS's hook uploads. Config and
/// attributes can lie about the latter, so the commits decide.
fn skips_lfs_upload(repo: &gix::Repository, target: &SafePushAfterCommitTarget) -> Result<bool> {
    if !runs_pre_push_hook(repo) {
        return Ok(false);
    }
    let head = super::object_id_from_commit_id(&target.local_head, repo.object_hash()).ok_or_else(
        || {
            Error::new(ErrorKind::Backend(format!(
                "invalid pushed commit id {}",
                target.local_head
            )))
        },
    )?;
    let pushed = commits_the_remote_has(repo, &target.remote, &target.branch)?;
    super::large_files::introduces_lfs_pointers(repo, head, pushed)
}

/// Commits `remote` already has, as far as a push of `branch` needs: the
/// branch's tracking ref and the remote's default branch, a merge's usual
/// source; for a new branch, every tracking ref its fetch refspecs write (a
/// name prefix would also take a remote named `<remote>/x`). Hiding fewer
/// only scans more, so a ref that does not resolve, such as a pruned
/// `origin/HEAD`, is left out. Hiding thousands of refs costs ~0.4 s.
fn commits_the_remote_has(
    repo: &gix::Repository,
    remote: &str,
    branch: &str,
) -> Result<Vec<gix::ObjectId>> {
    let refs = crate::refs::view(repo)?;
    let resolve = |name: &[u8]| {
        let mut reference = refs.find(name).ok().flatten()?;
        Some(reference.peel_to_id().ok()?.detach())
    };
    let upstream = Upstream {
        remote: remote.to_string(),
        branch: branch.to_string(),
    };
    let mut commits: Vec<_> = tracking_refs_for_remote_branch(repo, &upstream)?
        .iter()
        .filter_map(|name| resolve(name.as_bytes()))
        .collect();
    if commits.is_empty() {
        let owned: FxHashSet<_> =
            tracking_refs_of_remote(repo, &refs, remote, &CancellationToken::new())?
                .into_iter()
                .collect();
        commits.extend(
            refs.remote_branches()?
                .peeled()
                .flatten()
                .filter(|reference| owned.contains(reference.name()))
                .map(|reference| reference.id().detach()),
        );
    } else {
        commits.extend(resolve(format!("refs/remotes/{remote}/HEAD").as_bytes()));
    }
    commits.retain(|id| repo.has_object(*id));
    Ok(commits)
}

/// Whether Git runs a `pre-push` hook here, all that `--no-verify` skips on a
/// push: one configured for the event, or an executable `pre-push` in the
/// hooks directory. Disabled configured hooks count too; that only scans more.
fn runs_pre_push_hook(repo: &gix::Repository) -> bool {
    let config = repo.config_snapshot();
    let configured = config
        .plumbing()
        .sections_by_name("hook")
        .into_iter()
        .flatten()
        .any(|section| {
            section
                .values("event")
                .iter()
                .any(|event| event == "pre-push")
        });
    // A relative `core.hooksPath` is relative to where hooks run.
    let hooks = match config.trusted_path("core.hooksPath") {
        Ok(Some(path)) => repo.workdir().unwrap_or(repo.git_dir()).join(path),
        Ok(None) => repo.common_dir().join("hooks"),
        Err(_) => return true,
    };
    configured || is_hook_program(&hooks.join("pre-push"))
}

/// Git's `find_hook`: an executable file, which Windows reads as any file,
/// with or without `.exe`.
#[cfg(unix)]
fn is_hook_program(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_hook_program(path: &std::path::Path) -> bool {
    path.exists() || path.with_extension("exe").exists()
}

fn safe_push_ref_display(remote: &str, branch: &str) -> String {
    format!("{remote}/{branch}")
}

impl GixRepo {
    pub(super) fn push_set_upstream_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
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
        let output = run_git_with_output(cmd, &command_label)?;
        if let Ok(Some(local_branch)) = self.current_branch_name() {
            self.clear_pending_upstream_if_matches(
                &local_branch,
                &Upstream {
                    remote: remote.to_string(),
                    branch: branch.to_string(),
                },
            );
        }
        Ok(output)
    }

    fn push_head_to_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
        force_with_lease: bool,
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
        run_git_with_output(cmd, &command_label)
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

        // Built-in force push and multi-ref publication use the same exact
        // lease validation and command builder. The source is the captured
        // commit, so a branch moving after the checks cannot alter the push.
        self.push_refs_impl(
            &lease.remote,
            &[gitcomet_core::services::RemoteRefUpdate {
                branch: lease.branch.clone(),
                expected: lease.expected.clone(),
                new: lease.local_head.clone(),
            }],
            false,
        )
    }

    pub(super) fn head_commit_id_impl(&self) -> Result<Option<CommitId>> {
        let repo = self.reopen_repo()?;
        gix_head_id_or_none(&repo).map(|id| id.map(|id| CommitId(oid_to_arc_str(&id))))
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

    fn push_after_commit_target_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
        set_upstream: bool,
        hooks: GitHooks,
    ) -> Result<CommandOutput> {
        self.validate_push_after_commit_target(target)?;
        // Git LFS uploads objects from its `pre-push` hook, and `--no-verify`
        // skips that hook too. Upload them first, as the hook would have, so
        // the pushed commits never point at objects the server lacks.
        if !hooks.skipped() || !skips_lfs_upload(&self.reopen_repo()?, target)? {
            return self.push_after_commit_target_once(target, set_upstream, hooks);
        }
        // Both commands answer the same credentials prompt.
        crate::util::with_shared_git_auth(|| {
            let local_head = target.local_head.to_string();
            let mut cmd = self.git_workdir_cmd();
            cmd.args(["lfs", "push"])
                .arg(&target.remote)
                .arg(&local_head);
            let label = format!("git lfs push {} {local_head}", target.remote);
            let uploaded = run_git_with_output(cmd, &label)?;
            let pushed = self.push_after_commit_target_once(target, set_upstream, hooks)?;
            let label = format!("{label} && {}", pushed.command);
            Ok(super::lfs::combine(&label, vec![uploaded, pushed]))
        })
    }

    fn push_after_commit_target_once(
        &self,
        target: &SafePushAfterCommitTarget,
        set_upstream: bool,
        hooks: GitHooks,
    ) -> Result<CommandOutput> {
        let source = target.local_head.to_string();
        let refspec = format!("{source}:refs/heads/{}", target.branch);
        let flags = if hooks.skipped() { " --no-verify" } else { "" };
        let command_label = format!(
            "git push{flags} {} {}:refs/heads/{}",
            target.remote, source, target.branch
        );

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        if hooks.skipped() {
            cmd.arg("--no-verify");
        }
        cmd.arg("--").arg(&target.remote).arg(refspec);
        let mut output = run_git_with_output(cmd, &command_label)?;
        let upstream = Upstream {
            remote: target.remote.clone(),
            branch: target.branch.clone(),
        };
        if !set_upstream {
            self.clear_pending_upstream_if_matches(&target.local_branch, &upstream);
        } else if let Err(error) = self.reopen_repo().and_then(|repo| {
            // What `--set-upstream` would write, which a commit source skips.
            write_branch_upstream_config(
                &repo,
                &target.local_branch,
                &upstream,
                UpstreamBranch::Published,
            )
        }) {
            // Like `git push --set-upstream`: the push stands and says what failed.
            if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
                output.stderr.push('\n');
            }
            output.stderr.push_str(&format!(
                "{UPSTREAM_CONFIG_WRITE_FAILED}: {error}\n\
                 hint: run `git branch --set-upstream-to={}/{} {}`\n",
                upstream.remote, upstream.branch, target.local_branch
            ));
        }
        Ok(output)
    }

    pub(super) fn push_after_commit_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_output_impl(target, false, GitHooks::Run)
    }

    pub(super) fn push_after_commit_set_upstream_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_output_impl(target, true, GitHooks::Run)
    }

    pub(super) fn push_after_commit_with_hooks_impl(
        &self,
        target: &SafePushAfterCommitTarget,
        set_upstream: bool,
        hooks: GitHooks,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_output_impl(target, set_upstream, hooks)
    }

    fn fetch_remote_branch_tip_with_output(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<Option<(CommitId, CommandOutput)>> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let remote_ref = format!("refs/heads/{branch}");
        let label = format!("git fetch --no-prune --refmap= {remote} {remote_ref}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("fetch")
            .arg("--no-prune")
            .arg("--no-prune-tags")
            .arg("--no-tags")
            .arg("--refmap=")
            .arg("--")
            .arg(remote)
            .arg(&remote_ref);
        let output = run_git_raw_output(cmd, &label)?;
        if !output.status.success() {
            if self.remote_confirms_ref_missing(remote, &remote_ref) {
                return Ok(None);
            }
            return Err(git_command_failed_error(&label, output));
        }
        let fetch_output = command_output(&label, &output);

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
        Ok(Some((tip, fetch_output)))
    }

    /// Exit status 2 is the remote's own answer that it has no such ref; any
    /// other outcome leaves the failed fetch to speak for itself.
    fn remote_confirms_ref_missing(&self, remote: &str, remote_ref: &str) -> bool {
        let label = format!("git ls-remote --exit-code {remote} {remote_ref}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("ls-remote")
            .arg("--exit-code")
            .arg("--")
            .arg(remote)
            .arg(remote_ref);
        run_git_raw_output(cmd, &label).is_ok_and(|output| output.status.code() == Some(2))
    }

    fn fetch_remote_branch_tip_for_safe_push(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<Option<CommitId>> {
        Ok(self
            .fetch_remote_branch_tip_with_output(remote, branch)?
            .map(|(tip, _)| tip))
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

        if let Some(upstream) = self.configured_branch_upstream(local_branch)? {
            let has_live_upstream = self.branch_upstream(local_branch)?.is_some();
            return self.safe_push_decision_for_target(
                context,
                local_branch,
                upstream.remote,
                upstream.branch,
                has_live_upstream,
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

    pub(super) fn push_with_output_impl(&self) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()? {
            if let Some(upstream) = self.configured_branch_upstream(&branch)? {
                let output = self.push_head_to_branch_with_output_impl(
                    &upstream.remote,
                    &upstream.branch,
                    false,
                )?;
                self.clear_pending_upstream_if_matches(&branch, &upstream);
                return Ok(output);
            }

            if let Some(remote) = self.preferred_remote_name()? {
                return self.push_set_upstream_with_output_impl(&remote, &branch);
            }
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        run_git_with_output(cmd, "git push")
    }

    pub(super) fn push_force_with_output_impl(&self) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()?
            && let Some(upstream) = self.branch_upstream(&branch)?
        {
            let output = self.push_head_to_branch_with_output_impl(
                &upstream.remote,
                &upstream.branch,
                true,
            )?;
            self.clear_pending_upstream_if_matches(&branch, &upstream);
            return Ok(output);
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push").arg("--force-with-lease");
        run_git_with_output(cmd, "git push --force-with-lease")
    }

    pub(super) fn push_force_with_lease_with_output_impl(
        &self,
        lease: &ForcePushLease,
    ) -> Result<CommandOutput> {
        self.push_head_to_branch_with_oid_lease_with_output_impl(lease)
    }
}
