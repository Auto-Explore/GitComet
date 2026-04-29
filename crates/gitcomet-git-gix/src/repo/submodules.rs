use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::{
    bytes_to_text_preserving_utf8, git_workdir_cmd_for, path_buf_from_git_bytes,
    run_git_raw_output, run_git_simple, run_git_with_output,
};
use gitcomet_core::domain::{
    CommitId, DiffTarget, FileStatus, RepoStatus, Submodule, SubmoduleDiffRange,
    SubmoduleDiffRangeKind, SubmoduleDiffSummary, SubmoduleDiffSummaryMode, SubmoduleInnerChange,
    SubmoduleStatus,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{
    CommandOutput, Result, SubmoduleTrustDecision, SubmoduleTrustTarget,
};
use gix::bstr::ByteSlice as _;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

type NumstatLineCounts = (Option<u32>, Option<u32>);
type NumstatCounts = BTreeMap<PathBuf, NumstatLineCounts>;

fn allow_file_submodule_transport(cmd: &mut Command) {
    // `git submodule` blocks local-path remotes unless `protocol.file.allow` is enabled.
    // Use per-command config so local workflows keep working without disabling `https`/`ssh`.
    cmd.arg("-c").arg("protocol.file.allow=always");
}

impl GixRepo {
    pub(super) fn list_submodules_impl(&self) -> Result<Vec<Submodule>> {
        let repo = self.reopen_repo()?;
        let mut submodules = Vec::new();
        collect_repo_submodules(&repo, Path::new(""), &mut submodules)?;
        submodules.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(submodules)
    }

    pub(super) fn submodule_diff_summary_impl(
        &self,
        target: &DiffTarget,
    ) -> Result<SubmoduleDiffSummary> {
        let repo = self.reopen_repo()?;
        match target {
            DiffTarget::WorkingTree { path, .. } => {
                submodule_worktree_diff_summary(&repo, &self.list_submodules_impl()?, path)
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
            } => submodule_commit_diff_summary(&repo, commit_id, path),
            _ => Err(Error::new(ErrorKind::Unsupported(
                "submodule summaries require a submodule working-tree target or committed submodule path",
            ))),
        }
    }

    pub(super) fn check_submodule_add_trust_impl(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let Some(target) =
            trust_target_from_raw_source(repo_workdir_for_submodule_trust(&repo), path, url)?
        else {
            return Ok(SubmoduleTrustDecision::Proceed);
        };

        if submodule_source_trusted(repo_workdir_for_submodule_trust(&repo), &target)? {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: vec![target],
            })
        }
    }

    pub(super) fn check_submodule_update_trust_impl(&self) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo);
        let mut sources = BTreeMap::new();
        collect_repo_untrusted_submodule_sources(&repo, trust_root, Path::new(""), &mut sources)?;
        if sources.is_empty() {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: sources.into_values().collect(),
            })
        }
    }

    pub(super) fn check_submodule_load_trust_impl(
        &self,
        path: &Path,
    ) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo);
        let mut sources = BTreeMap::new();
        let found = collect_target_submodule_untrusted_sources(
            &repo,
            trust_root,
            Path::new(""),
            path,
            &mut sources,
        )?;
        if !found {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not configured in this repository",
                path.display()
            ))));
        }
        if sources.is_empty() {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: sources.into_values().collect(),
            })
        }
    }

    pub(super) fn add_submodule_with_output_impl(
        &self,
        url: &str,
        path: &Path,
        branch: Option<&str>,
        name: Option<&str>,
        force: bool,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo);
        let git_dir = repo.git_dir().to_path_buf();
        persist_submodule_trust_approvals(trust_root, approved_sources)?;

        let mut cmd = self.git_workdir_cmd();
        if let Some(target) = trust_target_from_raw_source(trust_root, path, url)? {
            if !submodule_source_trusted(trust_root, &target)? {
                return Err(untrusted_local_submodule_error(&target, "add"));
            }
            allow_file_submodule_transport(&mut cmd);
        }
        let logical_name = name
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());

        cmd.arg("submodule").arg("add");
        let mut command = "git submodule add".to_string();
        if let Some(branch) = branch {
            cmd.arg("--branch").arg(branch);
            command.push_str(&format!(" --branch {branch}"));
        }
        if force {
            cmd.arg("--force");
            command.push_str(" --force");
        }
        if let Some(name) = name {
            cmd.arg("--name").arg(name);
            command.push_str(&format!(" --name {name}"));
        }
        cmd.arg(url).arg(path);
        command.push_str(&format!(" {url} {}", path.display()));
        match run_git_with_output(cmd, &command) {
            Ok(output) => Ok(output),
            Err(err) => Err(cleanup_failed_submodule_add_error(
                trust_root,
                &git_dir,
                path,
                &logical_name,
                err,
            )),
        }
    }

    pub(super) fn update_submodules_with_output_impl(
        &self,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo).to_path_buf();
        persist_submodule_trust_approvals(&trust_root, approved_sources)?;

        let mut outputs = Vec::new();
        update_repo_submodules_recursive(&repo, &trust_root, Path::new(""), &mut outputs)?;

        if outputs.is_empty() {
            Ok(CommandOutput::empty_success(
                "git submodule update --init --recursive",
            ))
        } else {
            Ok(combine_submodule_update_outputs(outputs))
        }
    }

    pub(super) fn load_submodule_with_output_impl(
        &self,
        path: &Path,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo).to_path_buf();
        persist_submodule_trust_approvals(&trust_root, approved_sources)?;

        let mut outputs = Vec::new();
        let found =
            load_target_submodule_recursive(&repo, &trust_root, Path::new(""), path, &mut outputs)?;
        if !found {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not configured in this repository",
                path.display()
            ))));
        }
        if outputs.is_empty() {
            Ok(CommandOutput::empty_success(format!(
                "git submodule update --init -- {}",
                path.display()
            )))
        } else {
            Ok(combine_command_outputs(
                format!("Load submodule {}", path.display()),
                outputs,
            ))
        }
    }

    pub(super) fn change_submodule_pointer_with_output_impl(
        &self,
        path: &Path,
        reference: &str,
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let Some(nested_repo) = open_gitlink_repo(&repo, path)? else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not initialized",
                path.display()
            ))));
        };

        let nested_workdir = repo_workdir_for_submodule_trust(&repo).join(path);
        let nested_status_repo =
            GixRepo::new(nested_workdir.clone(), nested_repo.clone().into_sync());
        let nested_status = nested_status_repo.status_impl()?;
        if !nested_status.staged.is_empty() || !nested_status.unstaged.is_empty() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' has inner changes. Commit, stash, or discard them before changing the pointer.",
                path.display()
            ))));
        }

        let target_commit = resolve_submodule_target_commit_id(&nested_repo, reference)?;
        let target_commit_hex = target_commit.to_string();

        let mut checkout_cmd = git_workdir_cmd_for(&nested_workdir);
        checkout_cmd
            .arg("checkout")
            .arg("--detach")
            .arg(target_commit_hex.as_str());
        let checkout_output = run_git_with_output(
            checkout_cmd,
            &format!("git checkout --detach {}", target_commit_hex),
        )?;

        let mut stage_cmd = self.git_workdir_cmd();
        stage_cmd.arg("add").arg("--").arg(path);
        let stage_output =
            run_git_with_output(stage_cmd, &format!("git add -- {}", path.display()))?;

        Ok(combine_command_outputs(
            format!("Change submodule pointer {}", path.display()),
            vec![checkout_output, stage_output],
        ))
    }

    pub(super) fn remove_submodule_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let workdir = repo_workdir_for_submodule_trust(&repo).to_path_buf();
        let git_dir = repo.git_dir().to_path_buf();
        let logical_name =
            resolve_submodule_logical_name(&repo, path)?.unwrap_or_else(|| path.to_path_buf());

        let mut cmd1 = self.git_workdir_cmd();
        cmd1.arg("submodule")
            .arg("deinit")
            .arg("-f")
            .arg("--")
            .arg(path);
        let out1 =
            run_git_with_output(cmd1, &format!("git submodule deinit -f {}", path.display()))?;

        let mut cmd2 = self.git_workdir_cmd();
        cmd2.arg("rm").arg("-f").arg("--").arg(path);
        let out2 = run_git_with_output(cmd2, &format!("git rm -f {}", path.display()))?;

        cleanup_removed_submodule_metadata(&workdir, &git_dir, &logical_name).map_err(|err| {
            Error::new(ErrorKind::Backend(format!(
                "Removed submodule '{}' from the worktree and index, but failed to clean metadata: {err}",
                path.display()
            )))
        })?;

        Ok(CommandOutput {
            command: format!("Remove submodule {}", path.display()),
            stdout: [out1.stdout.trim_end(), out2.stdout.trim_end()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            stderr: [out1.stderr.trim_end(), out2.stderr.trim_end()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: Some(0),
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct GitlinkIndexState {
    kind: Option<gix::hash::Kind>,
    index_id: Option<gix::ObjectId>,
    conflict: bool,
}

impl GitlinkIndexState {
    fn null_head(self, repo: &gix::Repository) -> CommitId {
        CommitId(
            self.kind
                .unwrap_or_else(|| repo.object_hash())
                .null()
                .to_string()
                .into(),
        )
    }

    fn index_head_or_null(self, repo: &gix::Repository) -> CommitId {
        self.index_id
            .map(object_id_to_commit_id)
            .unwrap_or_else(|| self.null_head(repo))
    }
}

fn collect_repo_submodules(
    repo: &gix::Repository,
    prefix: &Path,
    out: &mut Vec<Submodule>,
) -> Result<()> {
    let mut gitlinks = collect_gitlinks(repo)?;
    if let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    {
        for submodule in submodules {
            let relative_path = submodule
                .path()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
                .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
            let Some(gitlink) = gitlinks.remove(&relative_path) else {
                continue;
            };

            let full_path = prefix.join(&relative_path);
            let (row, nested_repo) =
                configured_submodule_row(repo, submodule, full_path.clone(), gitlink)?;
            out.push(row);
            if let Some(nested_repo) = nested_repo {
                collect_repo_submodules(&nested_repo, &full_path, out)?;
            }
        }
    }

    for (relative_path, gitlink) in gitlinks {
        let full_path = prefix.join(&relative_path);
        out.push(Submodule {
            path: full_path.clone(),
            recorded_head: gitlink.index_head_or_null(repo),
            checked_out_head: None,
            status: SubmoduleStatus::MissingMapping,
        });
        if let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)? {
            collect_repo_submodules(&nested_repo, &full_path, out)?;
        }
    }

    Ok(())
}

fn collect_repo_untrusted_submodule_sources(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
) -> Result<()> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(());
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        if let Some(target) = trust_target_from_submodule(current_workdir, &full_path, &submodule)?
            && !submodule_source_trusted(trust_root, &target)?
        {
            out.insert(full_path.clone(), target);
        }

        if let Some(nested_repo) = open_configured_submodule_repo(&submodule)? {
            collect_repo_untrusted_submodule_sources(&nested_repo, trust_root, &full_path, out)?;
        }
    }

    Ok(())
}

fn update_repo_submodules_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    outputs: &mut Vec<CommandOutput>,
) -> Result<()> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(());
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        let local_target = trust_target_from_submodule(current_workdir, &full_path, &submodule)?;

        let mut cmd = git_workdir_cmd_for(current_workdir);
        if let Some(target) = local_target.as_ref() {
            if !submodule_source_trusted(trust_root, target)? {
                return Err(untrusted_local_submodule_error(target, "update"));
            }
            allow_file_submodule_transport(&mut cmd);
        }

        cmd.arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--")
            .arg(&relative_path);
        outputs.push(run_git_with_output(
            cmd,
            &format!("git submodule update --init -- {}", full_path.display()),
        )?);

        if let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)? {
            update_repo_submodules_recursive(&nested_repo, trust_root, &full_path, outputs)?;
        }
    }

    Ok(())
}

fn collect_target_submodule_untrusted_sources(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    target_path: &Path,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
) -> Result<bool> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(false);
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        if full_path == target_path {
            if let Some(target) =
                trust_target_from_submodule(current_workdir, &full_path, &submodule)?
                && !submodule_source_trusted(trust_root, &target)?
            {
                out.insert(full_path.clone(), target);
            }
            if let Some(nested_repo) = open_configured_submodule_repo(&submodule)? {
                collect_repo_untrusted_submodule_sources(
                    &nested_repo,
                    trust_root,
                    &full_path,
                    out,
                )?;
            }
            return Ok(true);
        }

        if target_path.starts_with(&full_path)
            && let Some(nested_repo) = open_configured_submodule_repo(&submodule)?
            && collect_target_submodule_untrusted_sources(
                &nested_repo,
                trust_root,
                &full_path,
                target_path,
                out,
            )?
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn load_target_submodule_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    target_path: &Path,
    outputs: &mut Vec<CommandOutput>,
) -> Result<bool> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(false);
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        if full_path == target_path {
            let local_target =
                trust_target_from_submodule(current_workdir, &full_path, &submodule)?;
            let mut cmd = git_workdir_cmd_for(current_workdir);
            if let Some(target) = local_target.as_ref() {
                if !submodule_source_trusted(trust_root, target)? {
                    return Err(untrusted_local_submodule_error(target, "update"));
                }
                allow_file_submodule_transport(&mut cmd);
            }

            cmd.arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--")
                .arg(&relative_path);
            outputs.push(run_git_with_output(
                cmd,
                &format!("git submodule update --init -- {}", full_path.display()),
            )?);

            if let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)? {
                update_repo_submodules_recursive(&nested_repo, trust_root, &full_path, outputs)?;
            }
            return Ok(true);
        }

        if target_path.starts_with(&full_path)
            && let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)?
            && load_target_submodule_recursive(
                &nested_repo,
                trust_root,
                &full_path,
                target_path,
                outputs,
            )?
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn configured_submodule_row(
    repo: &gix::Repository,
    submodule: gix::Submodule<'_>,
    full_path: PathBuf,
    gitlink: GitlinkIndexState,
) -> Result<(Submodule, Option<gix::Repository>)> {
    if gitlink.conflict {
        return Ok((
            Submodule {
                path: full_path,
                recorded_head: gitlink.null_head(repo),
                checked_out_head: None,
                status: SubmoduleStatus::MergeConflict,
            },
            None,
        ));
    }

    let nested_repo = open_configured_submodule_repo(&submodule)?;
    let Some(nested_repo) = nested_repo else {
        return Ok((
            Submodule {
                path: full_path,
                recorded_head: gitlink.index_head_or_null(repo),
                checked_out_head: None,
                status: SubmoduleStatus::NotInitialized,
            },
            None,
        ));
    };

    let checked_out_head_id = gix_head_id_or_none(&nested_repo)?;
    let status = if checked_out_head_id == gitlink.index_id {
        SubmoduleStatus::UpToDate
    } else {
        SubmoduleStatus::HeadMismatch
    };
    let head = checked_out_head_id
        .map(object_id_to_commit_id)
        .unwrap_or_else(|| gitlink.null_head(repo));

    Ok((
        Submodule {
            path: full_path,
            recorded_head: gitlink.index_head_or_null(repo),
            checked_out_head: Some(head),
            status,
        },
        Some(nested_repo),
    ))
}

fn submodule_worktree_diff_summary(
    repo: &gix::Repository,
    submodules: &[Submodule],
    path: &Path,
) -> Result<SubmoduleDiffSummary> {
    let submodule = submodules
        .iter()
        .find(|submodule| submodule.path == path)
        .cloned()
        .ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not configured in this repository",
                path.display()
            )))
        })?;

    let nested_workdir = repo_workdir_for_submodule_trust(repo).join(&submodule.path);
    let nested_repo = open_gitlink_repo(repo, &submodule.path)?;
    let (live_staged, live_unstaged) =
        submodule_live_inner_changes(&nested_workdir, nested_repo.as_ref())?;

    let head_gitlink = Some(head_gitlink_commit_id_or_null(repo, &submodule.path)?);
    let index_gitlink = Some(submodule.recorded_head.clone());
    let checked_out_head = submodule.checked_out_head.clone();
    let not_loaded_reason = (nested_repo.is_none() || checked_out_head.is_none())
        .then_some("Submodule is not loaded locally.".to_string());

    let ranges = vec![
        build_submodule_range(
            &nested_workdir,
            nested_repo.as_ref(),
            SubmoduleDiffRangeKind::StagedPointer,
            head_gitlink.clone(),
            index_gitlink.clone(),
            None,
        )?,
        build_submodule_range(
            &nested_workdir,
            nested_repo.as_ref(),
            SubmoduleDiffRangeKind::UnstagedPointer,
            index_gitlink.clone(),
            checked_out_head.clone(),
            not_loaded_reason,
        )?,
    ];

    Ok(SubmoduleDiffSummary {
        path: submodule.path,
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(submodule.status),
        commit_id: None,
        parent_commit_id: None,
        checked_out_head,
        ranges,
        live_staged,
        live_unstaged,
    })
}

fn submodule_commit_diff_summary(
    repo: &gix::Repository,
    commit_id: &CommitId,
    path: &Path,
) -> Result<SubmoduleDiffSummary> {
    let parent_commit_id = first_parent_commit_id(repo, commit_id)?;
    let from = match parent_commit_id.as_ref() {
        Some(parent_commit_id) => {
            gitlink_commit_id_at_revision(repo, parent_commit_id.as_ref(), path)?
        }
        None => None,
    };
    let to = gitlink_commit_id_at_revision(repo, commit_id.as_ref(), path)?;

    let nested_workdir = repo_workdir_for_submodule_trust(repo).join(path);
    let nested_repo = open_gitlink_repo(repo, path)?;
    let unavailable_reason = if nested_repo.is_none() {
        Some("Submodule history is not available locally.".to_string())
    } else {
        None
    };

    let ranges = vec![build_submodule_range(
        &nested_workdir,
        nested_repo.as_ref(),
        SubmoduleDiffRangeKind::CommitHistory,
        from,
        to,
        unavailable_reason,
    )?];

    Ok(SubmoduleDiffSummary {
        path: path.to_path_buf(),
        mode: SubmoduleDiffSummaryMode::CommitHistory,
        status: None,
        commit_id: Some(commit_id.clone()),
        parent_commit_id,
        checked_out_head: None,
        ranges,
        live_staged: Vec::new(),
        live_unstaged: Vec::new(),
    })
}

fn submodule_live_inner_changes(
    nested_workdir: &Path,
    nested_repo: Option<&gix::Repository>,
) -> Result<(Vec<SubmoduleInnerChange>, Vec<SubmoduleInnerChange>)> {
    let Some(nested_repo) = nested_repo else {
        return Ok((Vec::new(), Vec::new()));
    };

    let nested_status_repo = GixRepo::new(
        nested_workdir.to_path_buf(),
        nested_repo.clone().into_sync(),
    );
    let RepoStatus { staged, unstaged } = nested_status_repo.status_impl()?;
    let staged_counts = git_numstat_counts(nested_workdir, true)?;
    let unstaged_counts = git_numstat_counts(nested_workdir, false)?;
    Ok((
        submodule_inner_changes_from_status(staged, &staged_counts),
        submodule_inner_changes_from_status(unstaged, &unstaged_counts),
    ))
}

fn build_submodule_range(
    nested_workdir: &Path,
    nested_repo: Option<&gix::Repository>,
    kind: SubmoduleDiffRangeKind,
    from: Option<CommitId>,
    to: Option<CommitId>,
    unavailable_reason: Option<String>,
) -> Result<SubmoduleDiffRange> {
    let changes = if unavailable_reason.is_none() {
        match (nested_repo, from.as_ref(), to.as_ref()) {
            (_, Some(from), Some(to)) if from == to => Vec::new(),
            (Some(_), Some(from), Some(to)) => {
                submodule_range_changes_from_commits(nested_workdir, from, to)?
            }
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let unavailable_reason = unavailable_reason.or_else(|| {
        if from.is_none() || to.is_none() {
            Some("Only one side of the submodule pointer is available.".to_string())
        } else if nested_repo.is_none() {
            Some("Submodule history is not available locally.".to_string())
        } else {
            None
        }
    });

    Ok(SubmoduleDiffRange {
        kind,
        from,
        to,
        changes,
        unavailable_reason,
    })
}

fn submodule_range_changes_from_commits(
    nested_workdir: &Path,
    from: &CommitId,
    to: &CommitId,
) -> Result<Vec<SubmoduleInnerChange>> {
    let status_changes = git_range_status_changes(nested_workdir, from, to)?;
    let counts = git_range_numstat_counts(nested_workdir, from, to)?;
    Ok(status_changes
        .into_iter()
        .map(|(path, kind)| {
            let (additions, deletions) = counts.get(&path).cloned().unwrap_or((None, None));
            SubmoduleInnerChange {
                path,
                kind,
                additions,
                deletions,
            }
        })
        .collect())
}

fn submodule_inner_changes_from_status(
    entries: Vec<FileStatus>,
    counts: &NumstatCounts,
) -> Vec<SubmoduleInnerChange> {
    entries
        .into_iter()
        .map(|entry| {
            let (additions, deletions) = counts.get(&entry.path).cloned().unwrap_or((None, None));
            SubmoduleInnerChange {
                path: entry.path,
                kind: entry.kind,
                additions,
                deletions,
            }
        })
        .collect()
}

fn parse_numstat_field(field: &[u8]) -> Option<u32> {
    if field == b"-" {
        return None;
    }
    std::str::from_utf8(field).ok()?.parse::<u32>().ok()
}

fn next_non_empty_nul_field<'a, I>(fields: &mut I) -> Option<&'a [u8]>
where
    I: Iterator<Item = &'a [u8]>,
{
    fields.find(|field| !field.is_empty())
}

fn git_numstat_counts(workdir: &Path, cached: bool) -> Result<NumstatCounts> {
    let mut command = git_workdir_cmd_for(workdir);
    command.arg("--no-optional-locks").arg("diff");
    if cached {
        command.arg("--cached");
    }
    command.arg("--numstat").arg("-z").arg("--no-renames");
    let label = if cached {
        "git diff --cached --numstat -z --no-renames"
    } else {
        "git diff --numstat -z --no-renames"
    };
    let output = run_git_raw_output(command, label)?;
    if !output.status.success() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{label} failed: {}",
            bytes_to_text_preserving_utf8(&output.stderr).trim()
        ))));
    }

    let mut counts = BTreeMap::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_field(fields.next().unwrap_or_default());
        let deletions = parse_numstat_field(fields.next().unwrap_or_default());
        let path =
            path_buf_from_git_bytes(fields.next().unwrap_or_default(), "git diff --numstat path")?;
        counts.insert(path, (additions, deletions));
    }

    Ok(counts)
}

fn git_range_status_changes(
    workdir: &Path,
    from: &CommitId,
    to: &CommitId,
) -> Result<Vec<(PathBuf, gitcomet_core::domain::FileStatusKind)>> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("diff")
        .arg("--name-status")
        .arg("-z")
        .arg("--find-renames")
        .arg(from.as_ref())
        .arg(to.as_ref());
    let label = "git diff --name-status -z --find-renames";
    let output = run_git_raw_output(command, label)?;
    if !output.status.success() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{label} failed: {}",
            bytes_to_text_preserving_utf8(&output.stderr).trim()
        ))));
    }

    let mut fields = output.stdout.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(status_field) = next_non_empty_nul_field(&mut fields) {
        let Some(status_code) = status_field.first().copied() else {
            continue;
        };
        let kind = match status_code {
            b'A' | b'C' => gitcomet_core::domain::FileStatusKind::Added,
            b'D' => gitcomet_core::domain::FileStatusKind::Deleted,
            b'R' => gitcomet_core::domain::FileStatusKind::Renamed,
            b'U' => gitcomet_core::domain::FileStatusKind::Conflicted,
            _ => gitcomet_core::domain::FileStatusKind::Modified,
        };

        let path_bytes = if matches!(status_code, b'R' | b'C') {
            let _old_path = next_non_empty_nul_field(&mut fields);
            next_non_empty_nul_field(&mut fields).unwrap_or_default()
        } else {
            next_non_empty_nul_field(&mut fields).unwrap_or_default()
        };

        if path_bytes.is_empty() {
            continue;
        }

        changes.push((
            path_buf_from_git_bytes(path_bytes, "git diff --name-status path")?,
            kind,
        ));
    }

    Ok(changes)
}

fn git_range_numstat_counts(
    workdir: &Path,
    from: &CommitId,
    to: &CommitId,
) -> Result<NumstatCounts> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("diff")
        .arg("--numstat")
        .arg("-z")
        .arg("--find-renames")
        .arg(from.as_ref())
        .arg(to.as_ref());
    let label = "git diff --numstat -z --find-renames";
    let output = run_git_raw_output(command, label)?;
    if !output.status.success() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{label} failed: {}",
            bytes_to_text_preserving_utf8(&output.stderr).trim()
        ))));
    }

    let mut counts = BTreeMap::new();
    let mut fields = output.stdout.split(|byte| *byte == 0);
    while let Some(record) = next_non_empty_nul_field(&mut fields) {
        let mut columns = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_field(columns.next().unwrap_or_default());
        let deletions = parse_numstat_field(columns.next().unwrap_or_default());
        let path_field = columns.next().unwrap_or_default();
        let path_bytes = if path_field.is_empty() {
            let _old_path = next_non_empty_nul_field(&mut fields);
            next_non_empty_nul_field(&mut fields).unwrap_or_default()
        } else {
            path_field
        };
        if path_bytes.is_empty() {
            continue;
        }
        counts.insert(
            path_buf_from_git_bytes(path_bytes, "git diff --numstat path")?,
            (additions, deletions),
        );
    }

    Ok(counts)
}

fn resolve_submodule_target_commit_id(
    repo: &gix::Repository,
    reference: &str,
) -> Result<gix::ObjectId> {
    let object = repo
        .rev_parse_single(reference)
        .map_err(|_| {
            Error::new(ErrorKind::Backend(format!(
                "submodule reference '{}' did not resolve to an object",
                reference
            )))
        })?
        .object()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "resolve submodule reference '{}': {e}",
                reference
            )))
        })?;
    let commit = object.peel_to_commit().map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "submodule reference '{}' does not point to a commit: {e}",
            reference
        )))
    })?;
    Ok(commit.id)
}

fn first_parent_commit_id(
    repo: &gix::Repository,
    commit_id: &CommitId,
) -> Result<Option<CommitId>> {
    let Some(object_id) = object_id_from_commit_id(commit_id) else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid commit id '{}'",
            commit_id.as_ref()
        ))));
    };
    let commit = repo
        .find_commit(object_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find commit: {e}"))))?;
    Ok(commit
        .parent_ids()
        .next()
        .map(|id| object_id_to_commit_id(id.detach())))
}

fn gitlink_commit_id_at_revision(
    repo: &gix::Repository,
    revision: &str,
    path: &Path,
) -> Result<Option<CommitId>> {
    let object_id = repo
        .rev_parse_single(revision)
        .map(|id| id.detach())
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "resolve revision '{revision}' for submodule '{}': {e}",
                path.display()
            )))
        })?;
    gitlink_commit_id_in_object(repo, object_id, path)
}

fn gitlink_commit_id_in_object(
    repo: &gix::Repository,
    object_id: gix::ObjectId,
    path: &Path,
) -> Result<Option<CommitId>> {
    let object = repo.find_object(object_id).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix find object {object_id}: {e}"
        )))
    })?;
    let tree = object.peel_to_tree().map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix peel tree for submodule '{}': {e}",
            path.display()
        )))
    })?;
    let Some(entry) = tree.lookup_entry_by_path(path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix lookup submodule '{}': {e}",
            path.display()
        )))
    })?
    else {
        return Ok(None);
    };
    if !entry.mode().is_commit() {
        return Ok(None);
    }
    Ok(Some(object_id_to_commit_id(entry.object_id())))
}

fn head_gitlink_commit_id_or_null(repo: &gix::Repository, path: &Path) -> Result<CommitId> {
    let Some(head_id) = gix_head_id_or_none(repo)? else {
        return Ok(CommitId(repo.object_hash().null().to_string().into()));
    };
    Ok(gitlink_commit_id_in_object(repo, head_id, path)?
        .unwrap_or_else(|| CommitId(repo.object_hash().null().to_string().into())))
}

fn resolve_submodule_logical_name(repo: &gix::Repository, path: &Path) -> Result<Option<PathBuf>> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(None);
    };

    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        if relative_path == path {
            return pathbuf_from_gix_path(submodule.name()).map(Some);
        }
    }

    Ok(None)
}

fn cleanup_failed_submodule_add_error(
    workdir: &Path,
    git_dir: &Path,
    path: &Path,
    logical_name: &Path,
    err: Error,
) -> Error {
    let clone_only_state = match failed_submodule_add_left_clone_only_state(workdir, path) {
        Ok(value) => value,
        Err(probe_err) => {
            return append_failed_submodule_add_note(
                err,
                &format!("GitComet could not inspect failed submodule add state: {probe_err}"),
            );
        }
    };

    if !clone_only_state {
        return err;
    }

    match cleanup_failed_submodule_add_leftovers(workdir, git_dir, path, logical_name) {
        Ok(()) => err,
        Err(cleanup_err) => append_failed_submodule_add_note(
            err,
            &format!("Cleanup after failed submodule add also failed: {cleanup_err}"),
        ),
    }
}

fn failed_submodule_add_left_clone_only_state(workdir: &Path, path: &Path) -> Result<bool> {
    let repo = gix::open(workdir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "open repo after failed submodule add {}: {e}",
            workdir.display()
        )))
    })?;
    Ok(!submodule_path_registered(&repo, path)?)
}

fn submodule_path_registered(repo: &gix::Repository, path: &Path) -> Result<bool> {
    if configured_submodule_path_exists(repo, path)? {
        return Ok(true);
    }
    Ok(collect_gitlinks(repo)?.contains_key(path))
}

fn configured_submodule_path_exists(repo: &gix::Repository, path: &Path) -> Result<bool> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(false);
    };

    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        if relative_path == path {
            return Ok(true);
        }
    }

    Ok(false)
}

fn cleanup_failed_submodule_add_leftovers(
    workdir: &Path,
    git_dir: &Path,
    path: &Path,
    logical_name: &Path,
) -> Result<()> {
    remove_failed_submodule_checkout(workdir, git_dir, path, logical_name)?;
    cleanup_removed_submodule_metadata(workdir, git_dir, logical_name)
}

fn cleanup_removed_submodule_metadata(
    workdir: &Path,
    git_dir: &Path,
    logical_name: &Path,
) -> Result<()> {
    remove_local_submodule_config_section_if_present(workdir, logical_name)?;
    remove_submodule_git_dir(git_dir, logical_name)?;
    Ok(())
}

fn remove_failed_submodule_checkout(
    workdir: &Path,
    git_dir: &Path,
    submodule_path: &Path,
    logical_name: &Path,
) -> Result<()> {
    let checkout_path = submodule_worktree_path(workdir, submodule_path)?;
    if !checkout_path.exists() {
        return Ok(());
    }

    let expected_git_dir = canonicalize_or_original(git_dir.join("modules").join(logical_name));
    let Some(actual_git_dir) = checkout_git_dir_reference(&checkout_path)? else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to remove failed submodule checkout {} because it is not linked to {}",
            checkout_path.display(),
            expected_git_dir.display()
        ))));
    };

    if actual_git_dir != expected_git_dir {
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to remove failed submodule checkout {} because it points to {} instead of {}",
            checkout_path.display(),
            actual_git_dir.display(),
            expected_git_dir.display()
        ))));
    }

    fs::remove_dir_all(&checkout_path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "remove failed submodule checkout {}: {e}",
            checkout_path.display()
        )))
    })
}

fn submodule_worktree_path(workdir: &Path, submodule_path: &Path) -> Result<PathBuf> {
    if submodule_path.is_absolute() {
        if submodule_path.starts_with(workdir) {
            return Ok(submodule_path.to_path_buf());
        }
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to clean failed submodule add outside repository workdir: {}",
            submodule_path.display()
        ))));
    }
    Ok(workdir.join(submodule_path))
}

fn checkout_git_dir_reference(checkout_path: &Path) -> Result<Option<PathBuf>> {
    let dot_git = checkout_path.join(".git");
    let metadata = match fs::metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::new(ErrorKind::Io(err.kind()))),
    };

    if metadata.is_dir() {
        return Ok(Some(canonicalize_or_original(dot_git)));
    }

    let bytes = fs::read(&dot_git).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    let text = bytes_to_text_preserving_utf8(&bytes);
    let Some(git_dir) = text.strip_prefix("gitdir:") else {
        return Ok(None);
    };

    let git_dir = PathBuf::from(git_dir.trim());
    let resolved = if git_dir.is_absolute() {
        git_dir
    } else {
        checkout_path.join(git_dir)
    };
    Ok(Some(canonicalize_or_original(resolved)))
}

fn append_failed_submodule_add_note(err: Error, note: &str) -> Error {
    match err.kind() {
        ErrorKind::Git(failure) => Error::new(ErrorKind::Git(GitFailure::new(
            failure.command(),
            failure.id(),
            failure.exit_code(),
            failure.stdout().to_vec(),
            failure.stderr().to_vec(),
            Some(match failure.detail() {
                Some(detail) if !detail.is_empty() => format!("{detail}\n\n{note}"),
                _ => note.to_string(),
            }),
        ))),
        _ => Error::new(ErrorKind::Backend(format!("{err}\n\n{note}"))),
    }
}

fn remove_local_submodule_config_section_if_present(
    workdir: &Path,
    logical_name: &Path,
) -> Result<()> {
    let Some(logical_name) = logical_name.to_str() else {
        return Err(Error::new(ErrorKind::Unsupported(
            "submodule logical name is not valid UTF-8",
        )));
    };
    let section = format!("submodule.{logical_name}");

    let mut cmd = git_workdir_cmd_for(workdir);
    cmd.arg("config")
        .arg("--local")
        .arg("--remove-section")
        .arg(&section);
    let output = cmd
        .output()
        .map_err(|err| Error::new(ErrorKind::Io(err.kind())))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = bytes_to_text_preserving_utf8(&output.stderr);
    if stderr.contains("no such section") {
        return Ok(());
    }

    Err(Error::new(ErrorKind::Backend(format!(
        "git config --local --remove-section {section} failed: {}",
        stderr.trim()
    ))))
}

fn remove_submodule_git_dir(git_dir: &Path, logical_name: &Path) -> Result<()> {
    let modules_root = git_dir.join("modules");
    let module_dir = modules_root.join(logical_name);
    if !module_dir.exists() {
        return Ok(());
    }

    fs::remove_dir_all(&module_dir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "remove submodule git dir {}: {e}",
            module_dir.display()
        )))
    })?;
    prune_empty_module_parent_dirs(&modules_root, &module_dir)
}

fn prune_empty_module_parent_dirs(modules_root: &Path, removed_dir: &Path) -> Result<()> {
    let mut current = removed_dir.parent();
    while let Some(dir) = current {
        if dir == modules_root || !dir.starts_with(modules_root) {
            break;
        }

        let mut entries = fs::read_dir(dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read module metadata dir {}: {e}",
                dir.display()
            )))
        })?;
        match entries.next() {
            None => {
                fs::remove_dir(dir).map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "remove empty module metadata dir {}: {e}",
                        dir.display()
                    )))
                })?;
                current = dir.parent();
            }
            Some(Ok(_)) => break,
            Some(Err(e)) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "read module metadata dir entry {}: {e}",
                    dir.display()
                ))));
            }
        }
    }
    Ok(())
}

fn collect_gitlinks(repo: &gix::Repository) -> Result<BTreeMap<PathBuf, GitlinkIndexState>> {
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let path_backing = index.path_backing();

    let mut gitlinks: BTreeMap<PathBuf, GitlinkIndexState> = BTreeMap::new();
    for entry in index.entries() {
        if entry.mode != gix::index::entry::Mode::COMMIT {
            continue;
        }

        let path = pathbuf_from_gix_path(entry.path_in(path_backing))?;
        let state = gitlinks.entry(path).or_default();
        state.kind.get_or_insert(entry.id.kind());
        if entry.stage() == gix::index::entry::Stage::Unconflicted {
            state.index_id = Some(entry.id);
        } else {
            state.conflict = true;
        }
    }

    Ok(gitlinks)
}

fn open_gitlink_repo(
    repo: &gix::Repository,
    relative_path: &Path,
) -> Result<Option<gix::Repository>> {
    let Some(workdir) = repo.workdir() else {
        return Ok(None);
    };
    let path = workdir.join(relative_path);

    match gix::open(&path) {
        Ok(repo) => Ok(Some(repo)),
        Err(gix::open::Error::NotARepository { .. }) => Ok(None),
        Err(gix::open::Error::Io(io)) if io.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(gix::open::Error::Io(io)) => Err(Error::new(ErrorKind::Io(io.kind()))),
        Err(e) => Err(Error::new(ErrorKind::Backend(format!(
            "gix open nested submodule repo {}: {e}",
            path.display()
        )))),
    }
}

fn open_configured_submodule_repo(
    submodule: &gix::Submodule<'_>,
) -> Result<Option<gix::Repository>> {
    let state = submodule
        .state()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule state: {e}"))))?;
    if !(state.repository_exists && state.worktree_checkout) {
        return Ok(None);
    }
    submodule
        .open()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule open: {e}"))))
}

fn trust_target_from_submodule(
    current_repo_workdir: &Path,
    full_submodule_path: &Path,
    submodule: &gix::Submodule<'_>,
) -> Result<Option<SubmoduleTrustTarget>> {
    let url = submodule
        .url()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule url: {e}"))))?;
    trust_target_from_url(current_repo_workdir, full_submodule_path, &url)
}

fn trust_target_from_raw_source(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    raw_source: &str,
) -> Result<Option<SubmoduleTrustTarget>> {
    let url = gix::url::parse(raw_source.as_bytes().as_bstr()).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "invalid submodule source {raw_source:?}: {e}"
        )))
    })?;
    let display_source = raw_source.trim().to_string();
    trust_target_from_parsed_url(current_repo_workdir, submodule_path, &url, display_source)
}

fn trust_target_from_url(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    url: &gix::Url,
) -> Result<Option<SubmoduleTrustTarget>> {
    let display_source = bytes_to_text_preserving_utf8(url.to_bstring().as_ref());
    trust_target_from_parsed_url(current_repo_workdir, submodule_path, url, display_source)
}

fn trust_target_from_parsed_url(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    url: &gix::Url,
    display_source: String,
) -> Result<Option<SubmoduleTrustTarget>> {
    if url.scheme != gix::url::Scheme::File {
        return Ok(None);
    }

    let local_source_path = canonicalize_or_original(resolve_local_file_transport_path(
        current_repo_workdir,
        url,
    )?);
    Ok(Some(SubmoduleTrustTarget {
        submodule_path: submodule_path.to_path_buf(),
        display_source,
        local_source_path,
    }))
}

fn resolve_local_file_transport_path(
    current_repo_workdir: &Path,
    url: &gix::Url,
) -> Result<PathBuf> {
    let mut path = pathbuf_from_gix_path(url.path.as_ref())?;
    if let Some(host) = url.host.as_deref()
        && !host.eq_ignore_ascii_case("localhost")
    {
        let host_path = PathBuf::from(format!("//{host}")).join(&path);
        path = host_path;
    }
    if path.is_relative() {
        path = current_repo_workdir.join(path);
    }
    Ok(path)
}

fn persist_submodule_trust_approvals(
    trust_root: &Path,
    approved_sources: &[SubmoduleTrustTarget],
) -> Result<()> {
    const GIT_CONFIG_LOCK_RETRIES: usize = 6;

    for source in approved_sources {
        let key = submodule_file_transport_consent_key(trust_root, &source.local_source_path);
        if git_config_get_bool_global(trust_root, &key)?.unwrap_or(false) {
            continue;
        }

        let mut last_err = None;
        for attempt in 0..GIT_CONFIG_LOCK_RETRIES {
            let mut cmd = git_workdir_cmd_for(trust_root);
            cmd.arg("config").arg("--global").arg(&key).arg("true");
            match run_git_simple(cmd, &format!("git config --global {key} true")) {
                Ok(()) => {
                    last_err = None;
                    break;
                }
                Err(err) => {
                    if git_config_get_bool_global(trust_root, &key)?.unwrap_or(false) {
                        last_err = None;
                        break;
                    }
                    let retryable =
                        attempt + 1 < GIT_CONFIG_LOCK_RETRIES && is_git_config_lock_error(&err);
                    last_err = Some(err);
                    if retryable {
                        thread::sleep(Duration::from_millis(25));
                        continue;
                    }
                    break;
                }
            }
        }
        if let Some(err) = last_err {
            return Err(err);
        }
    }
    Ok(())
}

fn is_git_config_lock_error(err: &Error) -> bool {
    let text = match err.kind() {
        ErrorKind::Git(failure) => format!(
            "{}{}{}",
            failure.detail().unwrap_or_default(),
            String::from_utf8_lossy(failure.stderr()),
            String::from_utf8_lossy(failure.stdout())
        ),
        _ => err.to_string(),
    };
    text.contains("could not lock config file")
}

fn submodule_source_trusted(trust_root: &Path, source: &SubmoduleTrustTarget) -> Result<bool> {
    let key = submodule_file_transport_consent_key(trust_root, &source.local_source_path);
    Ok(git_config_get_bool_global(trust_root, &key)?.unwrap_or(false))
}

fn untrusted_local_submodule_error(source: &SubmoduleTrustTarget, action: &str) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "Refusing to {action} local submodule '{}' from '{}'. Explicit trust is required before enabling file transport.",
        source.submodule_path.display(),
        source.display_source
    )))
}

fn combine_submodule_update_outputs(outputs: Vec<CommandOutput>) -> CommandOutput {
    combine_command_outputs(
        "git submodule update --init --recursive".to_string(),
        outputs,
    )
}

fn combine_command_outputs(command: String, outputs: Vec<CommandOutput>) -> CommandOutput {
    CommandOutput {
        command,
        stdout: outputs
            .iter()
            .map(|output| output.stdout.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        stderr: outputs
            .iter()
            .map(|output| output.stderr.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        exit_code: Some(0),
    }
}

fn repo_workdir_for_submodule_trust(repo: &gix::Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.git_dir())
}

fn submodule_file_transport_consent_key(trust_root: &Path, source_path: &Path) -> String {
    let root = canonicalize_or_original(trust_root.to_path_buf());
    let source = canonicalize_or_original(source_path.to_path_buf());

    let mut bytes = stable_path_bytes(&root);
    bytes.push(0);
    bytes.extend_from_slice(&stable_path_bytes(&source));
    format!(
        "gitcomet.submodule.allowfiletransport-{:016x}",
        fnv1a_64(&bytes)
    )
}

fn git_config_get_bool_global(trust_root: &Path, key: &str) -> Result<Option<bool>> {
    let mut cmd = git_workdir_cmd_for(trust_root);
    cmd.arg("config")
        .arg("--global")
        .arg("--type=bool")
        .arg("--get")
        .arg(key);

    let output = cmd
        .output()
        .map_err(|err| Error::new(ErrorKind::Io(err.kind())))?;

    if output.status.success() {
        let value = bytes_to_text_preserving_utf8(&output.stdout);
        return match value.trim() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            other => Err(Error::new(ErrorKind::Backend(format!(
                "Invalid boolean value for git config {key}: {:?}. Expected true or false.",
                other
            )))),
        };
    }

    if output.status.code() == Some(1) {
        return Ok(None);
    }

    Err(Error::new(ErrorKind::Backend(format!(
        "git config --global --type=bool --get {key} failed: {}",
        bytes_to_text_preserving_utf8(&output.stderr).trim()
    ))))
}

fn stable_path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        path.as_os_str().as_bytes().to_vec()
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.to_str()
            .map(|text| text.as_bytes().to_vec())
            .unwrap_or_else(|| format!("{path:?}").into_bytes())
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn pathbuf_from_gix_path(path: &gix::bstr::BStr) -> Result<PathBuf> {
    gix::path::try_from_bstr(path)
        .map(|path| path.into_owned())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))
}

fn object_id_from_commit_id(id: &CommitId) -> Option<gix::ObjectId> {
    gix::ObjectId::from_hex(id.as_ref().as_bytes()).ok()
}

fn object_id_to_commit_id(id: gix::ObjectId) -> CommitId {
    CommitId(id.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::allow_file_submodule_transport;
    use super::submodule_file_transport_consent_key;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn allow_file_submodule_transport_uses_git_config_not_protocol_allowlist() {
        let mut cmd = Command::new("git");

        allow_file_submodule_transport(&mut cmd);

        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args, ["-c", "protocol.file.allow=always"]);
        assert!(
            !cmd.get_envs()
                .any(|(key, _)| key == OsStr::new("GIT_ALLOW_PROTOCOL"))
        );
    }

    #[test]
    fn consent_key_depends_on_root_and_source_path() {
        let a = submodule_file_transport_consent_key(
            Path::new("/repo-a"),
            Path::new("/sources/local-one"),
        );
        let b = submodule_file_transport_consent_key(
            Path::new("/repo-a"),
            Path::new("/sources/local-two"),
        );
        let c = submodule_file_transport_consent_key(
            Path::new("/repo-b"),
            Path::new("/sources/local-one"),
        );

        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
