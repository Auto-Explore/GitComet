use super::GixRepo;
use super::remotes::{
    NO_COLOR_CONFIG, remote_name_is_config_key_safe, tracking_refs_for_remote_branch,
};
use crate::util::{run_git_simple, validate_ref_like_arg};
use gitcomet_core::domain::Upstream;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, PullMode, Result};
use gitcomet_core::text_utils::{redact_url_userinfo, shell_single_quote};
use gix::bstr::ByteSlice as _;
use std::process::Command;

struct PullTarget {
    repository: String,
    source_ref: gix::refs::FullName,
}

impl PullTarget {
    fn new(repository: String, source_ref: String) -> Result<Self> {
        validate_ref_like_arg(&repository, "pull repository").map_err(|error| {
            Error::new(ErrorKind::Unsupported(
                "Pull requires a repository name or URL. Configure the branch's upstream before pulling.",
            ))
            .with_source(error)
        })?;
        if !source_ref.starts_with("refs/") {
            return Err(Error::new(ErrorKind::Unsupported(
                "Pull requires a full upstream ref. Configure the branch's upstream before pulling.",
            )));
        }
        let source_ref = gix::refs::FullName::try_from(source_ref).map_err(|error| {
            Error::new(ErrorKind::Unsupported(
                "Pull requires an exact upstream ref. Configure the branch's upstream before pulling.",
            ))
            .with_source(error)
        })?;
        Ok(Self {
            repository,
            source_ref,
        })
    }
}

fn pull_command_label(command: &Command) -> String {
    std::iter::once("git".to_string())
        .chain(command.get_args().map(|argument| {
            let argument = redact_url_userinfo(&argument.to_string_lossy());
            if !argument.is_empty()
                && argument
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_./:@%+=,-".contains(&byte))
            {
                argument
            } else {
                shell_single_quote(&argument)
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

fn automatic_upstream_tracking_ref(
    repo: &gix::Repository,
    target: &PullTarget,
    branch: &str,
) -> Result<String> {
    let tracking_refs = tracking_refs_for_remote_branch(
        repo,
        &Upstream {
            remote: target.repository.clone(),
            branch: branch.to_string(),
        },
    )?;
    let [tracking_ref] = tracking_refs.as_slice() else {
        return Err(Error::new(ErrorKind::Unsupported(
            "The pull source needs one tracking destination. Configure its fetch mapping and upstream before pulling.",
        )));
    };
    let null = repo.object_hash().null();
    for remote_name in repo.remote_names() {
        let remote = repo.find_remote(remote_name.as_bstr()).map_err(|error| {
            Error::new(ErrorKind::Unsupported(
                "Pull cannot resolve the tracking destination. Check the configured remote fetch mappings.",
            ))
            .with_source(error)
        })?;
        let matches = gix::refspec::MatchGroup::from_fetch_specs(
            remote
                .refspecs(gix::remote::Direction::Fetch)
                .iter()
                .map(|spec| spec.to_ref())
                .filter(|spec| spec.destination().is_some()),
        )
        .match_rhs(std::iter::once(gix::refspec::match_group::Item {
            full_ref_name: tracking_ref.as_bytes().as_bstr(),
            target: &null,
            object: None,
        }));
        for mapping in matches.mappings {
            let is_target = remote_name.as_bstr() == target.repository.as_bytes().as_bstr()
                && matches!(mapping.lhs,
                    gix::refspec::match_group::SourceRef::FullName(source)
                    if source.as_ref() == target.source_ref.as_bstr());
            if !is_target {
                return Err(Error::new(ErrorKind::Unsupported(
                    "The tracking destination belongs to multiple upstreams. Configure a unique fetch mapping and upstream before pulling.",
                )));
            }
        }
    }
    Ok(tracking_ref.clone())
}

impl GixRepo {
    fn configured_pull_target(&self, branch: &str) -> Result<Option<PullTarget>> {
        let repo = self.reopen_repo()?;
        let config = repo.config_snapshot();
        let config = config.plumbing();
        let subsection = Some(gix::bstr::BStr::new(branch));
        let repository = config.raw_value_by("branch", subsection, "remote").ok();
        let merges = config
            .raw_values_by("branch", subsection, "merge")
            .unwrap_or_default();
        if repository.is_none() && merges.is_empty() {
            return Ok(None);
        }
        let (Some(repository), Some(source_ref)) = (repository, merges.first()) else {
            return Err(Error::new(ErrorKind::Unsupported(
                "The upstream configuration is incomplete. Set an upstream before pulling.",
            )));
        };
        if merges.iter().any(|merge| merge != source_ref) {
            return Err(Error::new(ErrorKind::Unsupported(
                "Pull requires one upstream. Configure a single branch merge ref before pulling.",
            )));
        }
        let repository = repository.to_str().map_err(|error| {
            Error::new(ErrorKind::Unsupported("The pull repository must be UTF-8."))
                .with_source(error)
        })?;
        let source_ref = source_ref.to_str().map_err(|error| {
            Error::new(ErrorKind::Unsupported("The pull source ref must be UTF-8."))
                .with_source(error)
        })?;
        PullTarget::new(repository.to_string(), source_ref.to_string()).map(Some)
    }

    fn pull_command(&self, target: &PullTarget, mode: PullMode) -> Command {
        let mut command = self.git_workdir_cmd();
        command.args(NO_COLOR_CONFIG);
        command.args(["-c", "fetch.pruneTags=false"]);
        if remote_name_is_config_key_safe(&target.repository) && !target.repository.contains("://")
        {
            command
                .arg("-c")
                .arg(format!("remote.{}.pruneTags=false", target.repository));
        }
        command.args(["pull", "--progress"]);
        match mode {
            PullMode::Default | PullMode::FastForwardIfPossible => {
                command.arg("--ff");
            }
            PullMode::Merge => {
                command.args(["--no-rebase", "--ff"]);
            }
            PullMode::FastForwardOnly => {
                command.arg("--ff-only");
            }
            PullMode::Rebase => {
                command.arg("--rebase");
            }
        }
        command
            .args(["--no-prune", "--"])
            .arg(&target.repository)
            .arg(target.source_ref.as_bstr().to_string());
        command
    }

    fn pull_target(&self, target: &PullTarget, mode: PullMode) -> Result<CommandOutput> {
        let command = self.pull_command(target, mode);
        let label = pull_command_label(&command);
        if let Some(operation) = gitcomet_core::git_operation::current() {
            operation.emit(
                gitcomet_core::git_operation::GitOperationEvent::CommandInvocation {
                    command: label.clone(),
                },
            );
        }
        self.run_integrating_git(command, &label, true)
    }

    pub(super) fn pull_with_output_impl(&self, mode: PullMode) -> Result<CommandOutput> {
        let branch = self.current_branch_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Unsupported(
                "Pull requires a checked-out branch. Check out a branch before pulling.",
            ))
        })?;
        if let Some(target) = self.configured_pull_target(&branch)? {
            return self.pull_target(&target, mode);
        }

        let remote = self.preferred_remote_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Unsupported(
                "No remote is configured. Add a remote and set an upstream before pulling.",
            ))
        })?;
        let target = PullTarget::new(remote, format!("refs/heads/{branch}"))?;
        let repo = self.reopen_repo()?;
        let tracking_ref = automatic_upstream_tracking_ref(&repo, &target, &branch)?;

        let output = self.pull_target(&target, mode)?;
        let mut set_upstream = self.git_workdir_cmd();
        set_upstream
            .args(["branch", "--set-upstream-to"])
            .arg(tracking_ref)
            .arg("--")
            .arg(&branch);
        let label = pull_command_label(&set_upstream);
        run_git_simple(set_upstream, &label)?;
        Ok(output)
    }

    pub(super) fn pull_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(branch, "branch name")?;
        let target = PullTarget::new(remote.to_string(), format!("refs/heads/{branch}"))?;
        self.pull_target(&target, PullMode::Merge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_labels_quote_arguments_and_mask_url_credentials_without_changing_argv() {
        let mut command = Command::new("git");
        command.args([
            "-C",
            "/work/a b's repo",
            "pull",
            "--rebase",
            "--",
            "https://test-token@example.test/repo.git",
            "refs/heads/topic;echo",
        ]);
        assert_eq!(
            pull_command_label(&command),
            "git -C '/work/a b'\"'\"'s repo' pull --rebase -- 'https://***@example.test/repo.git' 'refs/heads/topic;echo'"
        );
        assert!(
            command
                .get_args()
                .any(|arg| arg == "https://test-token@example.test/repo.git")
        );
        command.arg("ssh://user:test-password@example.test/repo.git");
        let label = pull_command_label(&command);
        assert!(label.ends_with("'ssh://user:***@example.test/repo.git'"));
        assert!(!label.contains("test-password"));
    }
}
