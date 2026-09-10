use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_preview_output,
    run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use gitcomet_core::tag_push::{TagPushPreview, TagPushRequest};
use gitcomet_core::text_utils::redact_url_userinfo;
use std::collections::BTreeSet;
use std::process::{Command, Output};

impl GixRepo {
    fn tag_push_command(
        &self,
        request: &TagPushRequest,
        preview: bool,
    ) -> Result<(Command, String)> {
        validate_ref_like_arg(&request.remote, "remote name")?;
        validate_ref_like_arg(&request.branch, "branch name")?;
        validate_ref_like_arg(&request.local_branch, "local branch name")?;
        validate_hex_commit_id(&request.head)?;
        if self.current_branch_name()?.as_deref() != Some(&request.local_branch)
            || self.head_commit_id_impl()?.as_ref() != Some(&request.head)
        {
            return Err(Error::new(ErrorKind::Backend(
                "The current branch changed. Open Push again to update the destination and tag preview.".into(),
            )));
        }
        let mut cmd = self.git_workdir_cmd();
        if preview {
            // Credential helpers can open their own dialogs even when Git's
            // terminal prompts are disabled. Probes never invoke them; an
            // authenticated push remains available when a preview cannot run.
            cmd.args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.interactive=false",
            ]);
            cmd.env("GCM_INTERACTIVE", "Never");
        }
        cmd.arg("push").arg(request.mode.flag());
        if preview {
            // Hooks can have arbitrary side effects, even during a dry run.
            // Signing is also unnecessary for this read-only advertisement.
            cmd.args(["--dry-run", "--porcelain", "--no-verify", "--no-signed"]);
        } else if request.set_upstream {
            cmd.arg("--set-upstream");
        }
        let refspec = format!("HEAD:refs/heads/{}", request.branch);
        cmd.arg("--").arg(&request.remote).arg(&refspec);
        let label = format!(
            "git push {}{} {} {refspec}",
            request.mode.flag(),
            if preview {
                " --dry-run --porcelain --no-verify --no-signed"
            } else if request.set_upstream {
                " --set-upstream"
            } else {
                ""
            },
            redact_url_userinfo(&request.remote)
        );
        Ok((cmd, label))
    }

    pub(super) fn push_with_tags_impl(&self, request: &TagPushRequest) -> Result<CommandOutput> {
        let (cmd, label) = self.tag_push_command(request, false)?;
        let output = run_git_with_output(cmd, &label)?;
        self.clear_pending_upstream_if_matches(
            &request.local_branch,
            &gitcomet_core::domain::Upstream {
                remote: request.remote.clone(),
                branch: request.branch.clone(),
            },
        );
        Ok(output)
    }

    pub(super) fn preview_tag_push_impl(
        &self,
        request: &TagPushRequest,
        cancellation: &CancellationToken,
    ) -> Result<TagPushPreview> {
        cancellation.check_cancelled()?;
        let (cmd, label) = self.tag_push_command(request, true)?;
        let output = run_git_preview_output(cmd, &label, cancellation)?;
        parse_preview(output, &label)
    }
}

fn parse_preview(output: Output, label: &str) -> Result<TagPushPreview> {
    let mut preview = TagPushPreview::default();
    let mut new_tags = BTreeSet::new();
    let mut conflicts = BTreeSet::new();
    let mut rejected = BTreeSet::new();
    let mut saw_status = false;
    let mut destinations = 0;
    let mut completed = 0;
    for line in bytes_to_text_preserving_utf8(&output.stdout).lines() {
        if let Some(destination) = line.strip_prefix("To ") {
            destinations += 1;
            preview.destinations.push(redact_url_userinfo(destination));
            continue;
        }
        if line == "Done" {
            completed += 1;
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(flag), Some(refspec), Some(_summary)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some((_, target)) = refspec.split_once(':') else {
            continue;
        };
        saw_status = true;
        if let Some(tag) = target.strip_prefix("refs/tags/") {
            match flag {
                "*" => {
                    new_tags.insert(tag.to_string());
                }
                "!" => {
                    conflicts.insert(tag.to_string());
                }
                "=" => {}
                // Never silently count an unexpected tag update as a new tag.
                _ => {
                    conflicts.insert(tag.to_string());
                }
            }
        } else if flag == "!" {
            rejected.insert(
                target
                    .strip_prefix("refs/heads/")
                    .unwrap_or(target)
                    .to_string(),
            );
        }
    }
    // A rejected ref is still a useful complete preview. A transport failure,
    // including one failed URL of a multi-URL remote, is not a zero-tag result.
    if !saw_status
        || destinations == 0
        || completed != destinations
        || (!output.status.success() && conflicts.is_empty() && rejected.is_empty())
        || bytes_to_text_preserving_utf8(&output.stderr)
            .lines()
            .any(|line| line.starts_with("fatal:"))
    {
        return Err(git_command_failed_error(label, output));
    }
    preview.new_tags = new_tags.difference(&conflicts).cloned().collect();
    preview.conflicting_tags = conflicts.into_iter().collect();
    preview.rejected_branches = rejected.into_iter().collect();
    Ok(preview)
}
