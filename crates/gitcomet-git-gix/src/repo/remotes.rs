use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, run_git_capture, run_git_simple, run_git_with_output,
    validate_ref_like_arg,
};
use gitcomet_core::domain::{Remote, RemoteBranch};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, PullMode, RemoteUrlKind, Result};
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

    fn branch_has_upstream(&self, branch: &str) -> Result<bool> {
        validate_ref_like_arg(branch, "branch name")?;

        let repo = self.reopen_repo()?;
        let ref_name = format!("refs/heads/{branch}");
        let Some(reference) = repo
            .try_find_reference(ref_name.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
        else {
            return Ok(false);
        };

        Ok(matches!(
            reference.remote_tracking_ref_name(gix::remote::Direction::Fetch),
            Some(Ok(_))
        ))
    }

    pub(super) fn list_remotes_impl(&self) -> Result<Vec<Remote>> {
        let repo = self.reopen_repo()?;
        let mut remotes = Vec::new();

        for name in repo.remote_names() {
            let remote = repo.find_remote(name.as_ref()).map_err(|e| {
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
        let repo = self._repo.to_thread_local();
        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
        let iter = refs
            .remote_branches()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix remote_branches: {e}"))))?;

        let mut branches = Vec::new();
        for reference in iter {
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
                target: gitcomet_core::domain::CommitId(target.to_string()),
            });
        }

        branches.sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
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

    fn push_with_optional_output_impl(&self, capture_output: bool) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()?
            && !self.branch_has_upstream(&branch)?
            && let Some(remote) = self.preferred_remote_name()?
        {
            return self.push_set_upstream_with_optional_output_impl(
                &remote,
                &branch,
                capture_output,
            );
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
